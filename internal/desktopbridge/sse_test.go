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
