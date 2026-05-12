package desktopbridge

import (
	"context"
	"strings"
	"testing"

	"github.com/miopunch/miopunch/internal/task"
)

func TestReadLocalAPITaskEvents_ParsesMultipleEvents(t *testing.T) {
	t.Parallel()

	sse := strings.Join([]string{
		": keepalive",
		"data: {\"kind\":\"task.created\",\"task_id\":\"t1\"}",
		"",
		"data: {\"kind\":\"task.updated\",",
		"data: \"task_id\":\"t1\"}",
		"",
	}, "\n")

	var got []task.Event
	err := ReadLocalAPITaskEvents(context.Background(), strings.NewReader(sse), func(ev task.Event) error {
		got = append(got, ev)
		return nil
	})
	if err != nil {
		t.Fatalf("ReadLocalAPITaskEvents() error = %v", err)
	}
	if len(got) != 2 {
		t.Fatalf("events = %d, want %d", len(got), 2)
	}
	if got[0].Kind != "task.created" || got[0].TaskID != "t1" {
		t.Fatalf("event[0] = %+v, want kind/task_id", got[0])
	}
	if got[1].Kind != "task.updated" || got[1].TaskID != "t1" {
		t.Fatalf("event[1] = %+v, want kind/task_id", got[1])
	}
}

func TestReadLocalAPITaskEvents_RejectsNilHandler(t *testing.T) {
	t.Parallel()

	err := ReadLocalAPITaskEvents(context.Background(), strings.NewReader(""), nil)
	if err == nil {
		t.Fatalf("ReadLocalAPITaskEvents(nil handler) error = nil, want non-nil")
	}
}

func TestReadLocalAPIDesktopStateEvents_ParsesSnapshotAndRevisionedUpdate(t *testing.T) {
	t.Parallel()

	sse := strings.Join([]string{
		"data: {\"kind\":\"snapshot\",\"snapshot\":{\"rev\":4,\"tasks\":[],\"peer_sessions\":[],\"shell_sessions\":[],\"diagnostics\":[],\"approval_requests\":[],\"config\":{\"known_peers\":[]},\"topology\":{\"format\":\"miopunch.topology.v0\",\"members\":[],\"presence\":{},\"bootstrap\":{\"recommendations\":[],\"attempts\":[],\"more_rounds\":[]},\"neighbors\":{\"selected\":[],\"active\":[],\"degree_distribution\":[]},\"attempts\":[],\"payloads\":[],\"recovery\":{\"events\":[]}}}}",
		"",
		"data: {\"kind\":\"task.upsert\",\"base_rev\":4,\"rev\":5,\"task\":{\"task_id\":\"t1\",\"kind\":\"ping\",\"status\":\"running\",\"stage\":\"started\",\"created_at\":\"2026-05-12T00:00:00Z\",\"facts\":[],\"suggestions\":[]}}",
		"",
	}, "\n")

	var got []task.DesktopStateEvent
	err := ReadLocalAPIDesktopStateEvents(context.Background(), strings.NewReader(sse), func(ev task.DesktopStateEvent) error {
		got = append(got, ev)
		return nil
	})
	if err != nil {
		t.Fatalf("ReadLocalAPIDesktopStateEvents() error = %v", err)
	}
	if len(got) != 2 {
		t.Fatalf("events = %d, want %d", len(got), 2)
	}
	if got[0].Kind != task.DesktopStateEventSnapshot || got[0].Snapshot == nil || got[0].Snapshot.Rev != 4 {
		t.Fatalf("event[0] = %+v, want snapshot rev=4", got[0])
	}
	if got[1].Kind != task.DesktopStateEventTaskUpsert || got[1].BaseRev != 4 || got[1].Rev != 5 {
		t.Fatalf("event[1] = %+v, want task.upsert base_rev=4 rev=5", got[1])
	}
	if got[1].Task == nil || got[1].Task.ID != "t1" {
		t.Fatalf("event[1].Task = %+v, want task_id=t1", got[1].Task)
	}
}
