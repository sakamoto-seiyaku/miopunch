package localapi

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/task"
)

func TestSSE_Global_SnapshotFirst_AndKindField(t *testing.T) {
	t.Parallel()

	mgr := task.NewManager()
	srv := NewServer(ListenModeUser, mgr)
	ts := httptest.NewServer(srv.Handler())
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

	body := bytes.NewBufferString(`{"kind":"ping"}`)
	createResp := mustDoRequest(t, ctx, http.MethodPost, ts.URL+"/api/v0/tasks", body)
	if createResp.StatusCode != http.StatusCreated {
		b, _ := io.ReadAll(createResp.Body)
		_ = createResp.Body.Close()
		t.Fatalf("POST /api/v0/tasks status = %d, want %d, body=%s", createResp.StatusCode, http.StatusCreated, b)
	}
	_ = createResp.Body.Close()

	second := mustReadNextSSEData(t, r)
	assertJSONHasKind(t, second, "")
}

func TestSSE_Task_SnapshotFirst(t *testing.T) {
	t.Parallel()

	mgr := task.NewManager()
	srv := NewServer(ListenModeUser, mgr)
	ts := httptest.NewServer(srv.Handler())
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	t.Cleanup(cancel)

	body := bytes.NewBufferString(`{"kind":"ping"}`)
	createResp := mustDoRequest(t, ctx, http.MethodPost, ts.URL+"/api/v0/tasks", body)
	if createResp.StatusCode != http.StatusCreated {
		b, _ := io.ReadAll(createResp.Body)
		_ = createResp.Body.Close()
		t.Fatalf("POST /api/v0/tasks status = %d, want %d, body=%s", createResp.StatusCode, http.StatusCreated, b)
	}
	var created task.Task
	if err := json.NewDecoder(createResp.Body).Decode(&created); err != nil {
		_ = createResp.Body.Close()
		t.Fatalf("decode create task response: %v", err)
	}
	_ = createResp.Body.Close()

	resp := mustDoRequest(t, ctx, http.MethodGet, ts.URL+"/api/v0/tasks/"+created.ID+"/events", nil)
	if resp.StatusCode != http.StatusOK {
		b, _ := io.ReadAll(resp.Body)
		t.Fatalf("GET /api/v0/tasks/<id>/events status = %d, want %d, body=%s", resp.StatusCode, http.StatusOK, b)
	}
	t.Cleanup(func() { _ = resp.Body.Close() })

	r := bufio.NewReader(resp.Body)
	first := mustReadNextSSEData(t, r)
	assertJSONHasKind(t, first, "snapshot")

	var ev map[string]any
	if err := json.Unmarshal([]byte(first), &ev); err != nil {
		t.Fatalf("snapshot event not valid JSON: %v", err)
	}
	if _, ok := ev["task"]; !ok {
		t.Fatalf("snapshot event missing task field, got: %s", first)
	}
}

func mustDoRequest(t *testing.T, ctx context.Context, method string, url string, body io.Reader) *http.Response {
	t.Helper()

	req, err := http.NewRequestWithContext(ctx, method, url, body)
	if err != nil {
		t.Fatalf("new request: %v", err)
	}
	req.Host = poc.LocalAPIHost
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
