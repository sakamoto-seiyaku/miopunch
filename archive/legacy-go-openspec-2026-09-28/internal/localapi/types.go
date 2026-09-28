package localapi

import (
	"encoding/json"
	"time"

	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
)

type StatusResponse struct {
	Version   string     `json:"version"`
	StartedAt time.Time  `json:"started_at"`
	UptimeMs  int64      `json:"uptime_ms"`
	Mode      ListenMode `json:"mode"`
}

type Snapshot = pocruntime.Snapshot

type Event = pocruntime.Event

type ActionResult = pocruntime.ActionResult

type ActionRequest struct {
	Action string          `json:"action"`
	Args   json.RawMessage `json:"args,omitempty"`
}

// LogLevelRequest changes the daemon log level through LocalAPI.
type LogLevelRequest struct {
	LogLevel string `json:"log_level"`
}
