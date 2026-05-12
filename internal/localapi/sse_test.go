package localapi

import (
	"bufio"
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/256dpi/gomqtt/broker"
	"github.com/256dpi/gomqtt/transport"
	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
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

func TestDesktopStateRoute_ReturnsPersistedApprovalRequests(t *testing.T) {
	t.Parallel()

	statePath := filepath.Join(t.TempDir(), "state.json")
	requestMsgID := newLocalAPIMsgIDForTest(t)
	recordLocalAPIApprovalRequestForTest(t, statePath, requestMsgID)

	mgr := task.NewManagerWithStatePath(statePath)
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
	if len(snapshot.ApprovalRequests) != 1 {
		t.Fatalf("snapshot.ApprovalRequests length = %d, want 1", len(snapshot.ApprovalRequests))
	}
	got := snapshot.ApprovalRequests[0]
	if got.ApproveTaskID != "task-localapi-approve" {
		t.Errorf("ApprovalRequest.ApproveTaskID = %q, want %q", got.ApproveTaskID, "task-localapi-approve")
	}
	if got.RequestMsgID != requestMsgID {
		t.Errorf("ApprovalRequest.RequestMsgID = %q, want %q", got.RequestMsgID, requestMsgID)
	}
	if got.MemberPeerID != "peer-localapi-joiner" {
		t.Errorf("ApprovalRequest.MemberPeerID = %q, want %q", got.MemberPeerID, "peer-localapi-joiner")
	}
}

func TestDesktopSSE_StreamsApprovalRequestsReplace(t *testing.T) {
	t.Parallel()

	statePath := filepath.Join(t.TempDir(), "state.json")
	requestMsgID := newLocalAPIMsgIDForTest(t)
	recordLocalAPIApprovalRequestForTest(t, statePath, requestMsgID)

	mgr := task.NewManagerWithStatePath(statePath)
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
	if snapshotEvent.Snapshot == nil || len(snapshotEvent.Snapshot.ApprovalRequests) != 1 {
		t.Fatalf("snapshot approval request count = %d, want 1", len(snapshotEvent.Snapshot.ApprovalRequests))
	}

	body := bytes.NewBufferString(`{"kind":"approve","args":{}}`)
	createResp := mustDoRequest(t, ctx, http.MethodPost, ts.URL+"/api/v0/tasks", body)
	if createResp.StatusCode != http.StatusCreated {
		b, _ := io.ReadAll(createResp.Body)
		_ = createResp.Body.Close()
		t.Fatalf("POST /api/v0/tasks approve status = %d, want %d, body=%s", createResp.StatusCode, http.StatusCreated, b)
	}
	_ = createResp.Body.Close()

	update := mustReadDesktopSSEEventKind(t, r, task.DesktopStateEventApprovalRequestsReplace)
	if len(update.ApprovalRequests) != 1 {
		t.Fatalf("approval_requests.replace count = %d, want 1", len(update.ApprovalRequests))
	}
	if update.ApprovalRequests[0].RequestMsgID != requestMsgID {
		t.Fatalf("approval_requests.replace RequestMsgID = %q, want %q", update.ApprovalRequests[0].RequestMsgID, requestMsgID)
	}
}

func TestCreateTaskRoute_AllowsApproveDecision(t *testing.T) {
	t.Parallel()

	mgr := task.NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	t.Cleanup(mgr.Close)

	srv := NewServer(ListenModeUser, mgr)
	ts := httptest.NewServer(srv.Handler())
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	t.Cleanup(cancel)

	requestMsgID := newLocalAPIMsgIDForTest(t)
	body := bytes.NewBufferString(`{"kind":"approve_decision","args":{"approve_task_id":"task-localapi-approve","request_msg_id":"` + requestMsgID + `","decision":"approve"}}`)
	resp := mustDoRequest(t, ctx, http.MethodPost, ts.URL+"/api/v0/tasks", body)
	if resp.StatusCode != http.StatusCreated {
		b, _ := io.ReadAll(resp.Body)
		_ = resp.Body.Close()
		t.Fatalf("POST /api/v0/tasks approve_decision status = %d, want %d, body=%s", resp.StatusCode, http.StatusCreated, b)
	}
	defer func() { _ = resp.Body.Close() }()

	var created task.Task
	if err := json.NewDecoder(resp.Body).Decode(&created); err != nil {
		t.Fatalf("decode created approve_decision task: %v", err)
	}
	if created.Kind != "approve_decision" {
		t.Fatalf("created.Kind = %q, want approve_decision", created.Kind)
	}
}

func TestCreateTaskRoute_ApproveDecisionUsesPersistedRequestAfterRestart(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	brokerAddr := startLocalAPIMQTTBroker(t)
	approveTaskID, requestMsgID := recordLocalAPIPersistedApprovalDecisionForTest(t, statePath, brokerAddr)

	mgr := task.NewManagerWithStatePath(statePath)
	t.Cleanup(mgr.Close)

	srv := NewServer(ListenModeUser, mgr)
	ts := httptest.NewServer(srv.Handler())
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	t.Cleanup(cancel)

	body := bytes.NewBufferString(`{"kind":"approve_decision","args":{"approve_task_id":"` + approveTaskID + `","request_msg_id":"` + requestMsgID + `","decision":"reject"}}`)
	resp := mustDoRequest(t, ctx, http.MethodPost, ts.URL+"/api/v0/tasks", body)
	if resp.StatusCode != http.StatusCreated {
		b, _ := io.ReadAll(resp.Body)
		_ = resp.Body.Close()
		t.Fatalf("POST /api/v0/tasks approve_decision status = %d, want %d, body=%s", resp.StatusCode, http.StatusCreated, b)
	}
	var created task.Task
	if err := json.NewDecoder(resp.Body).Decode(&created); err != nil {
		_ = resp.Body.Close()
		t.Fatalf("decode created approve_decision task: %v", err)
	}
	_ = resp.Body.Close()

	final := waitLocalAPITaskDoneForTest(t, mgr, created.ID)
	if final.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("approve_decision ReasonCode = %q, want %q; facts=%v", final.ReasonCode, poc.ReasonCodeOK, final.Facts)
	}

	store := localAPIInviteStoreForTest(t, statePath)
	lookup, err := store.LookupApprovalRequest(approveTaskID, requestMsgID)
	if err != nil {
		t.Fatalf("LookupApprovalRequest() error = %v", err)
	}
	if lookup.Request.Status != controlplane.ApprovalStatusRejected {
		t.Fatalf("LookupApprovalRequest().Request.Status = %q, want %q", lookup.Request.Status, controlplane.ApprovalStatusRejected)
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

func newLocalAPIMsgIDForTest(t *testing.T) string {
	t.Helper()

	msgID, err := controlplane.NewMsgID()
	if err != nil {
		t.Fatalf("controlplane.NewMsgID() error = %v", err)
	}
	return msgID
}

func recordLocalAPIApprovalRequestForTest(t *testing.T, statePath string, requestMsgID string) {
	t.Helper()

	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	store, err := controlplane.NewInviteStore(stateDir)
	if err != nil {
		t.Fatalf("controlplane.NewInviteStore(%q) error = %v", stateDir, err)
	}
	if _, _, err := store.RecordApprovalRequest("miopunch/localapi/invite", time.Now().UTC().Add(time.Hour).UnixMilli(), 1, controlplane.ApprovalRequestRecord{
		ApproveTaskID: "task-localapi-approve",
		RequestMsgID:  requestMsgID,
		MemberPeerID:  "peer-localapi-joiner",
		MemberName:    "LocalAPI joiner",
		PlatformHint:  "linux",
	}); err != nil {
		t.Fatalf("RecordApprovalRequest() error = %v", err)
	}
}

func recordLocalAPIPersistedApprovalDecisionForTest(t *testing.T, statePath string, brokerAddr string) (string, string) {
	t.Helper()

	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			TopicPrefix: "miopunch/localapi",
			DataProto:   "quic",
			QUICCC:      "bbr",
		},
		Peers: map[string]pocstate.PeerConfig{},
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}
	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	if _, err := pocstate.EnsureIdentity(stateDir); err != nil {
		t.Fatalf("pocstate.EnsureIdentity(%q) error = %v", stateDir, err)
	}
	memberID, err := pocstate.EnsureIdentity(t.TempDir())
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(member) error = %v", err)
	}
	requestMsgID := newLocalAPIMsgIDForTest(t)
	body := map[string]any{
		"reply_topic":     "miopunch/localapi/reply",
		"member_name":     "LocalAPI restart joiner",
		"platform":        "linux",
		"ed25519_pub_b64": memberID.Ed25519PubB64(),
		"x25519_pub_b64":  memberID.X25519PubB64(),
	}
	bodyJSON, err := json.Marshal(body)
	if err != nil {
		t.Fatalf("json.Marshal(join request body) error = %v", err)
	}

	approveTaskID := "task-localapi-restart-approve"
	store := localAPIInviteStoreForTest(t, statePath)
	if _, _, err := store.RecordApprovalRequest("miopunch/localapi/restart-invite", time.Now().UTC().Add(time.Hour).UnixMilli(), 1, controlplane.ApprovalRequestRecord{
		ApproveTaskID: approveTaskID,
		RequestMsgID:  requestMsgID,
		MemberPeerID:  memberID.PeerID,
		MemberName:    "LocalAPI restart joiner",
		PlatformHint:  "linux",
		DecisionMaterial: &controlplane.ApprovalDecisionMaterial{
			InviteBrokers:                   []string{brokerAddr},
			ReplyTopic:                      "miopunch/localapi/reply",
			JoinRequestBodyB64URL:           base64.RawURLEncoding.EncodeToString(bodyJSON),
			MemberEd25519PubB64:             memberID.Ed25519PubB64(),
			MemberX25519PubB64:              memberID.X25519PubB64(),
			ValidatedAtUnixMs:               time.Now().UTC().UnixMilli(),
			ValidatedRequestExpiresAtUnixMs: time.Now().UTC().Add(time.Hour).UnixMilli(),
			ValidatedRequestSenderID:        memberID.PeerID,
		},
	}); err != nil {
		t.Fatalf("RecordApprovalRequest() error = %v", err)
	}
	return approveTaskID, requestMsgID
}

func localAPIInviteStoreForTest(t *testing.T, statePath string) *controlplane.InviteStore {
	t.Helper()

	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	store, err := controlplane.NewInviteStore(stateDir)
	if err != nil {
		t.Fatalf("controlplane.NewInviteStore(%q) error = %v", stateDir, err)
	}
	return store
}

func startLocalAPIMQTTBroker(t *testing.T) string {
	t.Helper()

	server, err := transport.Launch("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("transport.Launch(tcp://127.0.0.1:0) error = %v", err)
	}
	backend := broker.NewMemoryBackend()
	engine := broker.NewEngine(backend)
	engine.Accept(server)

	t.Cleanup(func() {
		_ = server.Close()
		backend.Close(500 * time.Millisecond)
		engine.Close()
	})

	return server.Addr().String()
}

func waitLocalAPITaskDoneForTest(t *testing.T, mgr *task.Manager, taskID string) task.Task {
	t.Helper()

	deadline := time.After(5 * time.Second)
	ticker := time.NewTicker(10 * time.Millisecond)
	defer ticker.Stop()
	for {
		if got, ok := mgr.Get(taskID); ok && got.Status == task.StatusDone {
			return got
		}
		select {
		case <-deadline:
			t.Fatalf("Manager.Get(%q) did not reach done", taskID)
		case <-ticker.C:
		}
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
