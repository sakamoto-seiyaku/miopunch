package task

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
)

func TestDoneEventIncludesTaskSnapshot(t *testing.T) {
	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	t.Cleanup(m.Close)

	sub := m.SubscribeAll()
	t.Cleanup(sub.Close)

	created, err := m.CreateAndRun(CreateRequest{Kind: "unsupported_snapshot_test"})
	if err != nil {
		t.Fatalf("CreateAndRun(unsupported_snapshot_test) error = %v", err)
	}

	timeout := time.After(2 * time.Second)
	for {
		select {
		case ev := <-sub.C:
			if ev.TaskID != created.ID || ev.Kind != "done" {
				continue
			}
			if ev.Task == nil {
				t.Fatalf("done event Task = nil, want snapshot")
			}
			if ev.Task.ID != created.ID {
				t.Fatalf("done event Task.ID = %q, want %q", ev.Task.ID, created.ID)
			}
			if ev.Task.Status != StatusDone {
				t.Fatalf("done event Task.Status = %q, want %q", ev.Task.Status, StatusDone)
			}
			if len(ev.Task.Facts) == 0 {
				t.Fatalf("done event Task.Facts length = 0, want final facts")
			}
			if len(ev.Task.Suggestions) == 0 {
				t.Fatalf("done event Task.Suggestions length = 0, want final suggestions")
			}
			return
		case <-timeout:
			t.Fatalf("timed out waiting for done event for task %s", created.ID)
		}
	}
}

func TestTaskUpdateEventsIncludeCurrentSnapshot(t *testing.T) {
	taskID := "task-snapshot-updates"
	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	t.Cleanup(m.Close)

	m.mu.Lock()
	m.tasks[taskID] = &Task{
		ID:          taskID,
		Kind:        "snapshot_test",
		CreatedAt:   time.Now().UTC(),
		Status:      StatusRunning,
		Stage:       poc.StageControlPlaneReady,
		Facts:       []poc.Fact{},
		Suggestions: []poc.Suggestion{},
	}
	m.mu.Unlock()

	sub := m.SubscribeTask(taskID)
	t.Cleanup(sub.Close)

	m.setStage(taskID, poc.StagePeerContact, "dial peer")
	stageEvent := readTaskEventForTest(t, sub.C, "stage")
	if stageEvent.Task == nil {
		t.Fatalf("stage event Task = nil, want snapshot")
	}
	if stageEvent.Task.Stage != poc.StagePeerContact {
		t.Fatalf("stage event Task.Stage = %q, want %q", stageEvent.Task.Stage, poc.StagePeerContact)
	}

	m.addFact(taskID, poc.Fact{TermID: "peer_id", Message: "peer_id=peer-test"})
	factEvent := readTaskEventForTest(t, sub.C, "fact")
	if factEvent.Task == nil {
		t.Fatalf("fact event Task = nil, want snapshot")
	}
	if !taskFactsContainSubstring(*factEvent.Task, "peer_id=peer-test") {
		t.Fatalf("fact event Task.Facts = %v, want peer_id fact", factEvent.Task.Facts)
	}

	m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
	diagnosisEvent := readTaskEventForTest(t, sub.C, "diagnosis")
	if diagnosisEvent.Task == nil {
		t.Fatalf("diagnosis event Task = nil, want snapshot")
	}
	if !taskSuggestionsContainSubstring(*diagnosisEvent.Task, "retry") {
		t.Fatalf("diagnosis event Task.Suggestions = %v, want retry suggestion", diagnosisEvent.Task.Suggestions)
	}

	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
	doneEvent := readTaskEventForTest(t, sub.C, "done")
	if doneEvent.Task == nil {
		t.Fatalf("done event Task = nil, want snapshot")
	}
	if doneEvent.Task.Status != StatusDone {
		t.Fatalf("done event Task.Status = %q, want %q", doneEvent.Task.Status, StatusDone)
	}
	if !doneEvent.Task.ReportReady {
		t.Fatalf("done event Task.ReportReady = false, want true")
	}
	if !taskFactsContainSubstring(*doneEvent.Task, "peer_id=peer-test") {
		t.Fatalf("done event Task.Facts = %v, want prior facts preserved", doneEvent.Task.Facts)
	}
	if !taskSuggestionsContainSubstring(*doneEvent.Task, "retry") {
		t.Fatalf("done event Task.Suggestions = %v, want prior suggestions preserved", doneEvent.Task.Suggestions)
	}
}

func TestDoneEventSnapshotIncludesReportPersistenceDiagnostics(t *testing.T) {
	const taskID = "task-report-persist-fails"

	m := NewManagerWithStatePath("")
	t.Cleanup(m.Close)

	m.mu.Lock()
	m.tasks[taskID] = &Task{
		ID:          taskID,
		Kind:        "snapshot_test",
		CreatedAt:   time.Now().UTC(),
		Status:      StatusRunning,
		Stage:       poc.StageControlPlaneReady,
		Facts:       []poc.Fact{},
		Suggestions: []poc.Suggestion{},
	}
	m.mu.Unlock()

	sub := m.SubscribeTask(taskID)
	t.Cleanup(sub.Close)

	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)

	doneEvent := readTaskEventForTest(t, sub.C, "done")
	if doneEvent.Task == nil {
		t.Fatalf("done event Task = nil, want snapshot")
	}
	if !taskFactsContainSubstring(*doneEvent.Task, "persist task report failed: empty state path") {
		t.Fatalf("done event Task.Facts = %v, want report persistence diagnostic", doneEvent.Task.Facts)
	}
	if !taskSuggestionsContainSubstring(*doneEvent.Task, "fix state_dir permissions") {
		t.Fatalf("done event Task.Suggestions = %v, want report persistence suggestion", doneEvent.Task.Suggestions)
	}
}

func readTaskEventForTest(t *testing.T, ch <-chan Event, kind string) Event {
	t.Helper()

	timeout := time.After(2 * time.Second)
	for {
		select {
		case ev, ok := <-ch:
			if !ok {
				t.Fatalf("task event channel closed while waiting for %q", kind)
			}
			if ev.Kind == kind {
				return ev
			}
		case <-timeout:
			t.Fatalf("timed out waiting for task event kind %q", kind)
		}
	}
}
