package task

import (
	"time"

	"github.com/miopunch/miopunch/internal/poc"
)

type Status string

const (
	StatusRunning Status = "running"
	StatusDone    Status = "done"
)

// Task is a long-running operation instance managed by the daemon.
type Task struct {
	ID        string    `json:"task_id"`
	Kind      string    `json:"kind"`
	CreatedAt time.Time `json:"created_at"`

	Status Status `json:"status"`

	Stage      poc.Stage      `json:"stage"`
	ReasonCode poc.ReasonCode `json:"reason_code,omitempty"`
	ExitCode   poc.ExitCode   `json:"exit_code,omitempty"`

	Facts       []poc.Fact       `json:"facts"`
	Suggestions []poc.Suggestion `json:"suggestions"`

	ReportReady bool `json:"report_ready"`

	Report string `json:"-"`

	Timeline []TimelineEntry `json:"-"`
}

func (t Task) Clone() Task {
	out := t
	if t.Facts != nil {
		out.Facts = append([]poc.Fact{}, t.Facts...)
	}
	if t.Suggestions != nil {
		out.Suggestions = append([]poc.Suggestion{}, t.Suggestions...)
	}
	return out
}
