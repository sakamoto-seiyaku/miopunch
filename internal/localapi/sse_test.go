package localapi

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"path/filepath"
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

func TestDesktopStateRoute_ReturnsSnapshotWithRevisionedState(t *testing.T) {
	t.Parallel()

	mgr := task.NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	t.Cleanup(mgr.Close)

	srv := NewServer(ListenModeUser, mgr)
	ts := httptest.NewServer(srv.Handler())
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	t.Cleanup(cancel)

	resp := mustDoRequest(t, ctx, http.MethodGet, ts.URL+"/api/v0/desktop/state", nil)
	if resp.StatusCode != http.StatusOK {
		b, _ := io.ReadAll(resp.Body)
		t.Fatalf("GET /api/v0/desktop/state status = %d, want %d, body=%s", resp.StatusCode, http.StatusOK, b)
	}
	defer func() { _ = resp.Body.Close() }()

	var snapshot task.DesktopStateSnapshot
	if err := json.NewDecoder(resp.Body).Decode(&snapshot); err != nil {
		t.Fatalf("decode desktop state snapshot: %v", err)
	}
	if snapshot.Status.Version == "" {
		t.Fatalf("snapshot.Status.Version = %q, want non-empty", snapshot.Status.Version)
	}
	if snapshot.Config.KnownPeers == nil {
		t.Fatalf("snapshot.Config.KnownPeers = nil, want empty slice")
	}
	if snapshot.PeerSessions == nil {
		t.Fatalf("snapshot.PeerSessions = nil, want empty slice")
	}
	if snapshot.ApprovalRequests == nil {
		t.Fatalf("snapshot.ApprovalRequests = nil, want empty slice")
	}
}

func TestDesktopSSE_SnapshotFirst_AndRevisionedDiagnosticsUpdate(t *testing.T) {
	t.Parallel()

	mgr := task.NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	t.Cleanup(mgr.Close)

	srv := NewServer(ListenModeUser, mgr)
	ts := httptest.NewServer(srv.Handler())
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	t.Cleanup(cancel)

	resp := mustDoRequest(t, ctx, http.MethodGet, ts.URL+"/api/v0/desktop/events", nil)
	if resp.StatusCode != http.StatusOK {
		b, _ := io.ReadAll(resp.Body)
		t.Fatalf("GET /api/v0/desktop/events status = %d, want %d, body=%s", resp.StatusCode, http.StatusOK, b)
	}
	t.Cleanup(func() { _ = resp.Body.Close() })

	r := bufio.NewReader(resp.Body)
	first := mustReadNextSSEData(t, r)

	var snapshotEvent task.DesktopStateEvent
	if err := json.Unmarshal([]byte(first), &snapshotEvent); err != nil {
		t.Fatalf("decode desktop snapshot event: %v", err)
	}
	if snapshotEvent.Kind != task.DesktopStateEventSnapshot {
		t.Fatalf("snapshot event kind = %q, want %q", snapshotEvent.Kind, task.DesktopStateEventSnapshot)
	}
	if snapshotEvent.Snapshot == nil {
		t.Fatalf("snapshot event Snapshot = nil, want non-nil")
	}

	if _, err := mgr.CreateAndRun(task.CreateRequest{Kind: "unsupported_snapshot_test"}); err != nil {
		t.Fatalf("CreateAndRun() error = %v", err)
	}

	update := mustReadDesktopSSEEventKind(t, r, task.DesktopStateEventDiagnosticsReplace)
	if update.BaseRev < snapshotEvent.Snapshot.Rev {
		t.Fatalf("update base_rev = %d, want >= snapshot rev %d", update.BaseRev, snapshotEvent.Snapshot.Rev)
	}
	if update.Rev <= update.BaseRev {
		t.Fatalf("update rev = %d, want > base_rev=%d", update.Rev, update.BaseRev)
	}
	if len(update.Diagnostics) == 0 {
		t.Fatalf("update Diagnostics length = 0, want refreshed diagnostics")
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

func mustReadDesktopSSEEventKind(t *testing.T, r *bufio.Reader, kind string) task.DesktopStateEvent {
	t.Helper()

	for i := 0; i < 20; i++ {
		raw := mustReadNextSSEData(t, r)
		var ev task.DesktopStateEvent
		if err := json.Unmarshal([]byte(raw), &ev); err != nil {
			t.Fatalf("decode desktop event: %v\nraw=%s", err, raw)
		}
		if ev.Kind == kind {
			return ev
		}
	}
	t.Fatalf("timed out waiting for desktop event kind %q", kind)
	return task.DesktopStateEvent{}
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
