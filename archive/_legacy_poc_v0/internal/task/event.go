package task

import (
	"time"

	"github.com/miopunch/miopunch/internal/poc"
)

// Event is an SSE-friendly JSON event body.
// The LocalAPI SSE endpoints stream these JSON objects as `data: <json>`.
//
// Task update events are coalesced state notifications, not a lossless event
// log. A slow subscriber may miss intermediate stage/fact/diagnosis events, so
// update events include the current Task snapshot when available. Clients that
// need reliable final task output should merge Event.Task or fetch the task by
// ID after observing completion.
type Event struct {
	Kind string `json:"kind"`

	TimeUnixMs int64  `json:"time_unix_ms,omitempty"`
	TaskID     string `json:"task_id,omitempty"`

	// Snapshot payloads (optional).
	Tasks []Task `json:"tasks,omitempty"`
	Task  *Task  `json:"task,omitempty"`

	Stage      poc.Stage      `json:"stage,omitempty"`
	ReasonCode poc.ReasonCode `json:"reason_code,omitempty"`
	ExitCode   poc.ExitCode   `json:"exit_code,omitempty"`

	Fact       *poc.Fact       `json:"fact,omitempty"`
	Suggestion *poc.Suggestion `json:"suggestion,omitempty"`
	Message    string          `json:"message,omitempty"`
}

func newEvent(kind string, taskID string) Event {
	return Event{
		Kind:       kind,
		TaskID:     taskID,
		TimeUnixMs: time.Now().UTC().UnixMilli(),
	}
}
