package localapi

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"runtime/debug"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/task"
)

type statusResponse struct {
	Version   string     `json:"version"`
	StartedAt time.Time  `json:"started_at"`
	UptimeMs  int64      `json:"uptime_ms"`
	Mode      ListenMode `json:"mode"`
}

func (s *Server) handleStatus(w http.ResponseWriter, r *http.Request) {
	now := time.Now().UTC()
	writeJSON(w, http.StatusOK, statusResponse{
		Version:   buildVersion(),
		StartedAt: s.startedAt,
		UptimeMs:  now.Sub(s.startedAt).Milliseconds(),
		Mode:      s.mode,
	})
}

type peersResponse struct {
	Peers []peer `json:"peers"`
}

type peer struct {
	PeerID string `json:"peer_id"`
}

func (s *Server) handlePeers(w http.ResponseWriter, r *http.Request) {
	peerIDs, err := s.tasks.ListPeers()
	if err != nil {
		reqID, _ := poc.NewRequestID()
		writeError(w, ErrorResponse{
			Stage:      "localapi",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Message:    "failed to load peers",
			Facts: []poc.Fact{
				{Message: "error=" + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
			RequestID: reqID,
		})
		return
	}

	out := make([]peer, 0, len(peerIDs))
	for _, peerID := range peerIDs {
		out = append(out, peer{PeerID: peerID})
	}
	writeJSON(w, http.StatusOK, peersResponse{Peers: out})
}

type tasksResponse struct {
	Tasks []task.Task `json:"tasks"`
}

func (s *Server) handleTasks(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, http.StatusOK, tasksResponse{Tasks: s.tasks.List()})
}

func (s *Server) handleTask(w http.ResponseWriter, r *http.Request) {
	rawTaskID := strings.TrimSpace(r.PathValue("task_id"))
	taskID, err := poc.CanonicalizeTaskID(rawTaskID)
	if err != nil {
		writeBadRequest(w, fmt.Sprintf("invalid task_id: %v", err))
		return
	}

	t, ok := s.tasks.Get(taskID)
	if !ok {
		writeNotFound(w, taskID, "task not found")
		return
	}
	writeJSON(w, http.StatusOK, t)
}

type createTaskRequest struct {
	Kind string          `json:"kind"`
	Args json.RawMessage `json:"args,omitempty"`
}

func (s *Server) handleCreateTask(w http.ResponseWriter, r *http.Request) {
	var req createTaskRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeBadRequest(w, "invalid JSON body")
		return
	}

	req.Kind = strings.TrimSpace(req.Kind)
	if !isSupportedTaskKind(req.Kind) {
		writeBadRequest(w, fmt.Sprintf("unsupported task kind: %q", req.Kind))
		return
	}

	t, err := s.tasks.CreateAndRun(task.CreateRequest{
		Kind: req.Kind,
		Args: req.Args,
	})
	if err != nil {
		writeBadRequest(w, err.Error())
		return
	}

	writeJSON(w, http.StatusCreated, t)
}

func (s *Server) handleTaskReport(w http.ResponseWriter, r *http.Request) {
	rawTaskID := strings.TrimSpace(r.PathValue("task_id"))
	taskID, err := poc.CanonicalizeTaskID(rawTaskID)
	if err != nil {
		writeBadRequest(w, fmt.Sprintf("invalid task_id: %v", err))
		return
	}

	if report, ok := s.tasks.GetReport(taskID); ok {
		w.Header().Set("Content-Type", "text/markdown; charset=utf-8")
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(report))
		return
	}

	if _, ok := s.tasks.Get(taskID); !ok {
		writeNotFound(w, taskID, "task not found")
		return
	}

	writeConflict(w, taskID, "report not ready")
}

func writeBadRequest(w http.ResponseWriter, message string) {
	reqID, _ := poc.NewRequestID()
	writeError(w, ErrorResponse{
		Stage:      "localapi",
		ReasonCode: poc.ReasonCodeBadRequest,
		ExitCode:   poc.ExitCodeBadRequest,
		Message:    message,
		Facts:      []poc.Fact{},
		Suggestions: []poc.Suggestion{
			{Message: "check request and retry"},
		},
		RequestID: reqID,
	})
}

func writeNotFound(w http.ResponseWriter, requestID string, message string) {
	writeError(w, ErrorResponse{
		Stage:      "localapi",
		ReasonCode: poc.ReasonCodeNotFound,
		ExitCode:   poc.ExitCodeNotFound,
		Message:    message,
		Facts: []poc.Fact{
			{TermID: "request_id", Message: "request_id=" + requestID},
		},
		Suggestions: []poc.Suggestion{
			{Message: "list tasks via: GET /api/v0/tasks"},
		},
		RequestID: requestID,
	})
}

func writeConflict(w http.ResponseWriter, requestID string, message string) {
	writeError(w, ErrorResponse{
		Stage:      "localapi",
		ReasonCode: poc.ReasonCodeConflict,
		ExitCode:   poc.ExitCodeConflict,
		Message:    message,
		Facts: []poc.Fact{
			{TermID: "request_id", Message: "request_id=" + requestID},
		},
		Suggestions: []poc.Suggestion{
			{Message: "wait for task completion"},
			{Message: "stream task events via: GET /api/v0/tasks/<task_id>/events"},
		},
		RequestID: requestID,
	})
}

func isSupportedTaskKind(kind string) bool {
	switch kind {
	case "invite", "join", "approve", "ping", "sh_ls", "sh_attach", "revoke_member":
		return true
	default:
		return false
	}
}

func buildVersion() string {
	info, ok := debug.ReadBuildInfo()
	if !ok {
		return "dev"
	}

	if info.Main.Version != "" && info.Main.Version != "(devel)" {
		return info.Main.Version
	}

	var revision string
	var modified string
	for _, s := range info.Settings {
		switch s.Key {
		case "vcs.revision":
			revision = s.Value
		case "vcs.modified":
			modified = s.Value
		}
	}
	if revision == "" {
		return "dev"
	}

	if len(revision) > 12 {
		revision = revision[:12]
	}
	if modified == "true" {
		return revision + "+dirty"
	}
	return revision
}

var errLocalAPINotImplemented = errors.New("not implemented")
