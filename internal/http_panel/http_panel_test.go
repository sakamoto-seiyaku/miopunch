package http_panel

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"

	"github.com/miopunch/miopunch/internal/task"
)

func TestCreateTask_KindWhitelist(t *testing.T) {
	t.Parallel()

	mgr := task.NewManager()
	t.Cleanup(mgr.Close)

	srv := NewServer("127.0.0.1:27400", mgr)
	h := srv.Handler()

	t.Run("reject_non_whitelisted", func(t *testing.T) {
		req := httptest.NewRequest(http.MethodPost, "http://127.0.0.1:27400/api/v0/tasks", bytes.NewBufferString(`{"kind":"ping"}`))
		req.Header.Set("Origin", "http://127.0.0.1:27400")

		rr := httptest.NewRecorder()
		h.ServeHTTP(rr, req)

		if rr.Code != http.StatusBadRequest {
			t.Fatalf("status=%d, want %d, body=%s", rr.Code, http.StatusBadRequest, rr.Body.String())
		}

		var resp ErrorResponse
		if err := json.NewDecoder(rr.Body).Decode(&resp); err != nil {
			t.Fatalf("decode error response: %v", err)
		}
		if !strings.Contains(resp.Message, "not allowed") {
			t.Fatalf("message=%q, want contains %q", resp.Message, "not allowed")
		}
		if len(resp.Suggestions) == 0 {
			t.Fatalf("missing suggestions, got=%+v", resp)
		}
	})

	for _, kind := range []string{"invite", "join", "sh_attach"} {
		kind := kind
		t.Run("allow_"+kind, func(t *testing.T) {
			t.Parallel()

			req := httptest.NewRequest(http.MethodPost, "http://127.0.0.1:27400/api/v0/tasks", bytes.NewBufferString(`{"kind":"`+kind+`"}`))
			req.Header.Set("Origin", "http://127.0.0.1:27400")

			rr := httptest.NewRecorder()
			h.ServeHTTP(rr, req)

			if rr.Code != http.StatusCreated {
				t.Fatalf("status=%d, want %d, body=%s", rr.Code, http.StatusCreated, rr.Body.String())
			}
		})
	}
}

func TestSameOriginEnforcement_POST(t *testing.T) {
	t.Parallel()

	mgr := task.NewManager()
	t.Cleanup(mgr.Close)

	srv := NewServer("127.0.0.1:27400", mgr)
	h := srv.Handler()

	t.Run("missing_origin", func(t *testing.T) {
		req := httptest.NewRequest(http.MethodPost, "http://127.0.0.1:27400/api/v0/tasks", bytes.NewBufferString(`{"kind":"invite"}`))

		rr := httptest.NewRecorder()
		h.ServeHTTP(rr, req)

		if rr.Code != http.StatusBadRequest {
			t.Fatalf("status=%d, want %d, body=%s", rr.Code, http.StatusBadRequest, rr.Body.String())
		}
		if !strings.Contains(rr.Body.String(), "same-origin") {
			t.Fatalf("body=%q, want contains %q", rr.Body.String(), "same-origin")
		}
	})

	t.Run("mismatch_origin", func(t *testing.T) {
		req := httptest.NewRequest(http.MethodPost, "http://127.0.0.1:27400/api/v0/tasks", bytes.NewBufferString(`{"kind":"invite"}`))
		req.Header.Set("Origin", "http://example.com")

		rr := httptest.NewRecorder()
		h.ServeHTTP(rr, req)

		if rr.Code != http.StatusBadRequest {
			t.Fatalf("status=%d, want %d, body=%s", rr.Code, http.StatusBadRequest, rr.Body.String())
		}
	})

	t.Run("localhost_alias_allowed", func(t *testing.T) {
		req := httptest.NewRequest(http.MethodPost, "http://127.0.0.1:27400/api/v0/tasks", bytes.NewBufferString(`{"kind":"invite"}`))
		req.Header.Set("Origin", "http://localhost:27400")

		rr := httptest.NewRecorder()
		h.ServeHTTP(rr, req)

		if rr.Code != http.StatusCreated {
			t.Fatalf("status=%d, want %d, body=%s", rr.Code, http.StatusCreated, rr.Body.String())
		}
	})
}

func TestSameOriginEnforcement_WS(t *testing.T) {
	t.Parallel()

	mgr := task.NewManager()
	t.Cleanup(mgr.Close)

	srv := NewServer("127.0.0.1:27400", mgr)
	h := srv.Handler()

	req := httptest.NewRequest(http.MethodGet, "http://127.0.0.1:27400/api/v0/tasks/task_00000000000000000000000000/ws", nil)
	rr := httptest.NewRecorder()
	h.ServeHTTP(rr, req)

	if rr.Code != http.StatusBadRequest {
		t.Fatalf("status=%d, want %d, body=%s", rr.Code, http.StatusBadRequest, rr.Body.String())
	}
	if !strings.Contains(rr.Body.String(), "same-origin") {
		t.Fatalf("body=%q, want contains %q", rr.Body.String(), "same-origin")
	}
}

func TestSSE_Global_SnapshotFirst(t *testing.T) {
	t.Parallel()

	mgr := task.NewManager()
	t.Cleanup(mgr.Close)

	ln, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	srv := NewServer(ln.Addr().String(), mgr)
	ts := httptest.NewUnstartedServer(srv.Handler())
	ts.Listener = ln
	ts.Start()
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	t.Cleanup(cancel)

	resp := mustDoRequest(t, ctx, http.MethodGet, ts.URL+"/api/v0/events", nil)
	if resp.StatusCode != http.StatusOK {
		b, _ := io.ReadAll(resp.Body)
		t.Fatalf("GET /api/v0/events status = %d, want %d, body=%s", resp.StatusCode, http.StatusOK, b)
	}
	t.Cleanup(func() { _ = resp.Body.Close() })

	r := bufio.NewReader(resp.Body)
	first := mustReadNextSSEData(t, r)
	assertJSONHasKind(t, first, "snapshot")
}

func TestWS_SubprotocolRequired(t *testing.T) {
	t.Parallel()

	mgr := task.NewManager()
	t.Cleanup(mgr.Close)

	ln, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	srv := NewServer(ln.Addr().String(), mgr)
	ts := httptest.NewUnstartedServer(srv.Handler())
	ts.Listener = ln
	ts.Start()
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	t.Cleanup(cancel)

	// Create a sh_attach task (same-origin required).
	body := bytes.NewBufferString(`{"kind":"sh_attach","args":{"peer_id":"peer1","session":"main"}}`)
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, ts.URL+"/api/v0/tasks", body)
	if err != nil {
		t.Fatalf("new request: %v", err)
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", ts.URL)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("POST /api/v0/tasks: %v", err)
	}
	t.Cleanup(func() { _ = resp.Body.Close() })
	if resp.StatusCode != http.StatusCreated {
		b, _ := io.ReadAll(resp.Body)
		t.Fatalf("POST /api/v0/tasks status=%d, want %d, body=%s", resp.StatusCode, http.StatusCreated, b)
	}

	var created task.Task
	if err := json.NewDecoder(resp.Body).Decode(&created); err != nil {
		t.Fatalf("decode create task response: %v", err)
	}
	if created.Kind != "sh_attach" {
		t.Fatalf("created kind=%q, want %q", created.Kind, "sh_attach")
	}

	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http") + "/api/v0/tasks/" + created.ID + "/ws"
	dialer := websocket.Dialer{}
	_, wsResp, err := dialer.DialContext(ctx, wsURL, http.Header{
		"Origin": []string{ts.URL},
	})
	if err == nil {
		t.Fatalf("DialContext(%q) error=nil, want non-nil", wsURL)
	}
	if wsResp == nil {
		t.Fatalf("DialContext(%q) resp=nil, want non-nil (handshake failure)", wsURL)
	}
	t.Cleanup(func() { _ = wsResp.Body.Close() })
	if wsResp.StatusCode != http.StatusBadRequest {
		b, _ := io.ReadAll(wsResp.Body)
		t.Fatalf("handshake status=%d, want %d, body=%s", wsResp.StatusCode, http.StatusBadRequest, b)
	}
}

func mustDoRequest(t *testing.T, ctx context.Context, method string, url string, body io.Reader) *http.Response {
	t.Helper()

	req, err := http.NewRequestWithContext(ctx, method, url, body)
	if err != nil {
		t.Fatalf("new request: %v", err)
	}
	return mustDo(t, req)
}

func mustDo(t *testing.T, req *http.Request) *http.Response {
	t.Helper()

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("do request: %v", err)
	}
	return resp
}

func mustReadNextSSEData(t *testing.T, r *bufio.Reader) string {
	t.Helper()

	var data string
	for {
		line, err := r.ReadString('\n')
		if err != nil {
			t.Fatalf("read SSE line: %v", err)
		}

		line = strings.TrimRight(line, "\r\n")
		if line == "" {
			if data != "" {
				return data
			}
			continue
		}

		if strings.HasPrefix(line, ":") {
			continue
		}
		if strings.HasPrefix(line, "data: ") {
			data = strings.TrimPrefix(line, "data: ")
		}
	}
}

func assertJSONHasKind(t *testing.T, b string, want string) {
	t.Helper()

	var v map[string]any
	if err := json.Unmarshal([]byte(b), &v); err != nil {
		t.Fatalf("event not valid JSON: %v\nraw=%s", err, b)
	}
	kind, ok := v["kind"].(string)
	if !ok {
		t.Fatalf("event missing kind string field, got: %s", b)
	}
	if want != "" && kind != want {
		t.Fatalf("event kind = %q, want %q\nraw=%s", kind, want, b)
	}
}
