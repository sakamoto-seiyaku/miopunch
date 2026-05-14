package task

import (
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

func TestSubscribeDesktopStateWithSnapshotChainsNextRevision(t *testing.T) {
	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	t.Cleanup(m.Close)

	sub, snapshot, err := m.SubscribeDesktopStateWithSnapshot()
	if err != nil {
		t.Fatalf("SubscribeDesktopStateWithSnapshot() error = %v", err)
	}
	t.Cleanup(sub.Close)
	assertDesktopRuntimeRevFact(t, snapshot.Diagnostics, snapshot.Rev)

	m.desktopMu.Lock()
	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:        DesktopStateEventDiagnosticsReplace,
		Diagnostics: cloneFacts(snapshot.Diagnostics),
	})
	m.desktopMu.Unlock()

	ev := readDesktopStateEventForTest(t, sub.C, DesktopStateEventDiagnosticsReplace)
	if ev.BaseRev != snapshot.Rev {
		t.Fatalf("desktop event BaseRev = %d, want snapshot Rev %d", ev.BaseRev, snapshot.Rev)
	}
	if ev.Rev <= ev.BaseRev {
		t.Fatalf("desktop event Rev = %d, want > BaseRev %d", ev.Rev, ev.BaseRev)
	}
}

func TestSubscribeDesktopStateWithSnapshotCleansUpOnSnapshotError(t *testing.T) {
	m := NewManagerWithStatePath("")
	t.Cleanup(m.Close)

	sub, _, err := m.SubscribeDesktopStateWithSnapshot()
	if err == nil {
		t.Fatalf("SubscribeDesktopStateWithSnapshot() error = nil, want non-nil")
	}
	if sub != nil {
		t.Fatalf("SubscribeDesktopStateWithSnapshot() subscription = non-nil, want nil")
	}

	m.desktopMu.Lock()
	got := len(m.desktopSubs)
	m.desktopMu.Unlock()
	if got != 0 {
		t.Fatalf("desktop subscriptions after snapshot error = %d, want 0", got)
	}
}

func TestPublishDesktopFromTaskEventPublishesDiagnosticsReplace(t *testing.T) {
	tests := []struct {
		name     string
		task     Task
		wantFact string
	}{
		{
			name:     "running task count",
			task:     desktopStateTestTask("task-desktop-diag-running", "snapshot_test", nil),
			wantFact: "running_tasks=1",
		},
		{
			name: "shell session count",
			task: desktopStateTestTask("task-desktop-diag-shell", "sh_attach", []poc.Fact{
				{TermID: "peer_id", Message: "peer_id=peer-test"},
				{TermID: "session", Message: "session=main"},
			}),
			wantFact: "shell_sessions=1",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
			t.Cleanup(m.Close)

			sub, _, err := m.SubscribeDesktopStateWithSnapshot()
			if err != nil {
				t.Fatalf("SubscribeDesktopStateWithSnapshot() error = %v", err)
			}
			t.Cleanup(sub.Close)

			taskSnapshot := tt.task.Clone()
			m.mu.Lock()
			m.tasks[taskSnapshot.ID] = &taskSnapshot
			m.mu.Unlock()

			eventTask := taskSnapshot.Clone()
			m.publishDesktopFromTaskEvent(Event{
				Kind:   "stage",
				TaskID: taskSnapshot.ID,
				Task:   &eventTask,
			})

			ev := readDesktopStateEventForTest(t, sub.C, DesktopStateEventDiagnosticsReplace)
			if !desktopFactsContain(ev.Diagnostics, tt.wantFact) {
				t.Fatalf("diagnostics = %v, want fact containing %q", ev.Diagnostics, tt.wantFact)
			}
			assertDesktopRuntimeRevFact(t, ev.Diagnostics, ev.Rev)
		})
	}
}

func TestBuildDesktopShellSessionsReportsAttachability(t *testing.T) {
	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	t.Cleanup(m.Close)

	waiting := desktopStateTestTask("task-waiting-shell", "sh_attach", []poc.Fact{
		{TermID: "peer_id", Message: "peer_id=peer-test"},
		{TermID: "target", Message: "target=local"},
		{TermID: "session", Message: "session=main"},
	})
	completed := desktopStateTestTask("task-completed-shell", "sh_attach", []poc.Fact{
		{TermID: "peer_id", Message: "peer_id=peer-test"},
	})
	completed.Status = StatusDone

	m.mu.Lock()
	m.attachByTask[waiting.ID] = &attachState{attachable: true}
	m.mu.Unlock()

	got := m.buildDesktopShellSessions([]Task{waiting, completed})
	if len(got) != 1 {
		t.Fatalf("buildDesktopShellSessions() length = %d, want 1: %#v", len(got), got)
	}
	if got[0].TaskID != waiting.ID {
		t.Fatalf("buildDesktopShellSessions()[0].TaskID = %q, want %q", got[0].TaskID, waiting.ID)
	}
	if !got[0].Attachable {
		t.Fatalf("buildDesktopShellSessions()[0].Attachable = false, want true")
	}
}

func TestSetShellAttachablePublishesShellSessionsReplace(t *testing.T) {
	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	t.Cleanup(m.Close)

	taskSnapshot := desktopStateTestTask("task-shell-attachable-event", "sh_attach", []poc.Fact{
		{TermID: "peer_id", Message: "peer_id=peer-test"},
		{TermID: "target", Message: "target=local"},
		{TermID: "session", Message: "session=main"},
	})
	m.mu.Lock()
	m.tasks[taskSnapshot.ID] = &taskSnapshot
	m.attachByTask[taskSnapshot.ID] = &attachState{attachable: true}
	m.mu.Unlock()

	sub, _, err := m.SubscribeDesktopStateWithSnapshot()
	if err != nil {
		t.Fatalf("SubscribeDesktopStateWithSnapshot() error = %v", err)
	}
	t.Cleanup(sub.Close)

	m.setShellAttachable(taskSnapshot.ID, false)

	ev := readDesktopStateEventForTest(t, sub.C, DesktopStateEventShellSessionsReplace)
	if len(ev.ShellSessions) != 1 {
		t.Fatalf("shell_sessions.replace length = %d, want 1: %#v", len(ev.ShellSessions), ev.ShellSessions)
	}
	if ev.ShellSessions[0].Attachable {
		t.Fatalf("shell_sessions.replace session Attachable = true, want false")
	}
}

func TestDesktopStateIncludesPersistedApprovalRequests(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	store, err := controlplane.NewInviteStore(stateDir)
	if err != nil {
		t.Fatalf("controlplane.NewInviteStore(%q) error = %v", stateDir, err)
	}
	expiresAt := time.Now().UTC().Add(time.Hour).UnixMilli()
	if _, _, err := store.RecordApprovalRequest("miopunch/test/invite", expiresAt, 1, controlplane.ApprovalRequestRecord{
		ApproveTaskID: "task-approve-review",
		RequestMsgID:  "request-msg-review",
		MemberPeerID:  "peer-reviewer",
		MemberName:    "Review laptop",
		PlatformHint:  "linux",
		V4Hint:        "easy",
	}); err != nil {
		t.Fatalf("RecordApprovalRequest() error = %v", err)
	}

	snapshot, err := m.DesktopStateSnapshot()
	if err != nil {
		t.Fatalf("DesktopStateSnapshot() error = %v", err)
	}
	if len(snapshot.ApprovalRequests) != 1 {
		t.Fatalf("DesktopStateSnapshot().ApprovalRequests length = %d, want 1", len(snapshot.ApprovalRequests))
	}
	got := snapshot.ApprovalRequests[0]
	if got.ApproveTaskID != "task-approve-review" {
		t.Errorf("ApprovalRequest.ApproveTaskID = %q, want %q", got.ApproveTaskID, "task-approve-review")
	}
	if got.RequestMsgID != "request-msg-review" {
		t.Errorf("ApprovalRequest.RequestMsgID = %q, want %q", got.RequestMsgID, "request-msg-review")
	}
	if got.MemberName != "Review laptop" {
		t.Errorf("ApprovalRequest.MemberName = %q, want %q", got.MemberName, "Review laptop")
	}
	if got.Status != controlplane.ApprovalStatusPending {
		t.Errorf("ApprovalRequest.Status = %q, want %q", got.Status, controlplane.ApprovalStatusPending)
	}
	if !desktopFactsContain(snapshot.Diagnostics, "approval_requests=1") {
		t.Fatalf("desktop diagnostics = %v, want approval_requests=1", snapshot.Diagnostics)
	}
}

func TestSaveStatePublishesDiagnosticsReplaceForOutgoingRevision(t *testing.T) {
	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	t.Cleanup(m.Close)

	sub, _, err := m.SubscribeDesktopStateWithSnapshot()
	if err != nil {
		t.Fatalf("SubscribeDesktopStateWithSnapshot() error = %v", err)
	}
	t.Cleanup(sub.Close)

	if err := m.saveState(pocstate.State{}); err != nil {
		t.Fatalf("saveState() error = %v", err)
	}

	ev := readDesktopStateEventForTest(t, sub.C, DesktopStateEventDiagnosticsReplace)
	assertDesktopRuntimeRevFact(t, ev.Diagnostics, ev.Rev)
}

func desktopStateTestTask(taskID string, kind string, facts []poc.Fact) Task {
	return Task{
		ID:          taskID,
		Kind:        kind,
		CreatedAt:   time.Now().UTC(),
		Status:      StatusRunning,
		Stage:       poc.StageControlPlaneReady,
		Facts:       append([]poc.Fact(nil), facts...),
		Suggestions: []poc.Suggestion{},
	}
}

func readDesktopStateEventForTest(t *testing.T, ch <-chan DesktopStateEvent, kind string) DesktopStateEvent {
	t.Helper()

	timeout := time.After(2 * time.Second)
	for {
		select {
		case ev, ok := <-ch:
			if !ok {
				t.Fatalf("desktop state event channel closed while waiting for %q", kind)
			}
			if ev.Kind == kind {
				return ev
			}
		case <-timeout:
			t.Fatalf("timed out waiting for desktop state event kind %q", kind)
		}
	}
}

func desktopFactsContain(facts []poc.Fact, needle string) bool {
	for _, fact := range facts {
		if strings.Contains(fact.Message, needle) {
			return true
		}
	}
	return false
}

func assertDesktopRuntimeRevFact(t *testing.T, facts []poc.Fact, wantRev uint64) {
	t.Helper()

	want := "desktop_runtime_rev=" + uint64String(wantRev)
	if !desktopFactsContain(facts, want) {
		t.Fatalf("desktop runtime diagnostics = %v, want fact containing %q", facts, want)
	}
}
