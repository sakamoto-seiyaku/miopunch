package http_panel

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
)

type Server struct {
	startedAt    time.Time
	listenAddr   string
	origin       string
	allowedHosts map[string]struct{}

	localAPIAddr localapi.Addr
}

func NewServer(listenAddr string, localAPIAddr localapi.Addr) *Server {
	origin, allowedHosts := panelOrigins(listenAddr)
	return &Server{
		startedAt:    time.Now().UTC(),
		listenAddr:   listenAddr,
		origin:       origin,
		allowedHosts: allowedHosts,
		localAPIAddr: localAPIAddr,
	}
}

func (s *Server) Origin() string {
	return s.origin
}

func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /", s.handleIndex)
	mux.HandleFunc("GET /api/v0/status", s.handleStatus)
	mux.HandleFunc("GET /api/v0/snapshot", s.handleSnapshot)
	mux.HandleFunc("POST /api/v0/action", s.handleAction)
	return mux
}

func panelOrigins(listenAddr string) (string, map[string]struct{}) {
	origin := "http://" + listenAddr
	allowedHosts := map[string]struct{}{
		listenAddr: {},
	}

	host, port, err := net.SplitHostPort(listenAddr)
	if err == nil && host == "127.0.0.1" {
		allowedHosts["localhost:"+port] = struct{}{}
	}

	return origin, allowedHosts
}

func (s *Server) handleIndex(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	_, _ = io.WriteString(w, `<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>miopunch runtime panel</title>
  <style>
    body { font-family: monospace; margin: 2rem; line-height: 1.4; }
    pre { background: #f5f5f5; padding: 1rem; overflow: auto; }
  </style>
</head>
<body>
  <h1>miopunch runtime panel</h1>
  <p>This panel is a thin LocalAPI v1 consumer.</p>
  <pre id="status">loading...</pre>
  <pre id="snapshot"></pre>
  <script>
    async function loadJSON(path) {
      const resp = await fetch(path);
      if (!resp.ok) throw new Error(path + " status " + resp.status);
      return await resp.json();
    }
    Promise.all([loadJSON("/api/v0/status"), loadJSON("/api/v0/snapshot")]).then(([status, snapshot]) => {
      document.getElementById("status").textContent = JSON.stringify(status, null, 2);
      document.getElementById("snapshot").textContent = JSON.stringify(snapshot, null, 2);
    }).catch((err) => {
      document.getElementById("status").textContent = String(err);
    });
  </script>
</body>
</html>`)
}

func (s *Server) handleStatus(w http.ResponseWriter, r *http.Request) {
	client, err := s.client()
	if err != nil {
		writeError(w, panelError("panel", poc.ReasonCodeInternal, poc.ExitCodeInternal, err.Error(), nil))
		return
	}
	status, err := client.GetStatus(r.Context())
	if err != nil {
		writePanelClientError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, status)
}

func (s *Server) handleSnapshot(w http.ResponseWriter, r *http.Request) {
	client, err := s.client()
	if err != nil {
		writeError(w, panelError("panel", poc.ReasonCodeInternal, poc.ExitCodeInternal, err.Error(), nil))
		return
	}
	snapshot, err := client.GetSnapshot(r.Context())
	if err != nil {
		writePanelClientError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, snapshot)
}

func (s *Server) handleAction(w http.ResponseWriter, r *http.Request) {
	if originErr := s.requireSameOrigin(r); originErr != nil {
		writeError(w, panelError(
			"panel",
			poc.ReasonCodeForbidden,
			poc.ExitCodeForbidden,
			originErr.Message,
			[]poc.Fact{
				{Message: "origin=" + strings.TrimSpace(originErr.OriginHeader)},
				{Message: "referer=" + strings.TrimSpace(originErr.RefererHeader)},
			},
		))
		return
	}

	client, err := s.client()
	if err != nil {
		writeError(w, panelError("panel", poc.ReasonCodeInternal, poc.ExitCodeInternal, err.Error(), nil))
		return
	}

	var req localapi.ActionRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeError(w, panelError("panel", poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invalid action request", []poc.Fact{{Message: "error=" + err.Error()}}))
		return
	}

	result, err := client.Action(r.Context(), req.Action, req.Args)
	if err != nil {
		writePanelClientError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, result)
}

func (s *Server) client() (*localapi.Client, error) {
	return localapi.NewClient(s.localAPIAddr)
}

func panelError(stage string, reasonCode poc.ReasonCode, exitCode poc.ExitCode, message string, facts []poc.Fact) ErrorResponse {
	return ErrorResponse{
		Stage:       stage,
		ReasonCode:  reasonCode,
		ExitCode:    exitCode,
		Message:     strings.TrimSpace(message),
		Facts:       append([]poc.Fact(nil), facts...),
		Suggestions: []poc.Suggestion{{Message: "retry"}},
	}
}

func writePanelClientError(w http.ResponseWriter, err error) {
	var apiErr *localapi.APIError
	if errors.As(err, &apiErr) {
		writeError(w, ErrorResponse{
			Stage:       apiErr.Response.Stage,
			ReasonCode:  apiErr.Response.ReasonCode,
			ExitCode:    apiErr.Response.ExitCode,
			Message:     apiErr.Response.Message,
			Facts:       apiErr.Response.Facts,
			Suggestions: apiErr.Response.Suggestions,
		})
		return
	}
	writeError(w, panelError("panel", poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, fmt.Sprintf("localapi request failed: %v", err), nil))
}
