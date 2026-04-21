package shellproto

import "time"

const (
	OpPing      = "ping"
	OpShLS      = "sh_ls"
	OpShAttach  = "sh_attach"
	OpWinSize   = "winsize"
	OpHeartbeat = "heartbeat"
)

const DefaultHeartbeatInterval = 15 * time.Second

type WinSize struct {
	Cols int `json:"cols"`
	Rows int `json:"rows"`
}

type ControlError struct {
	ReasonCode  string   `json:"reason_code,omitempty"`
	Message     string   `json:"message,omitempty"`
	Suggestions []string `json:"suggestions,omitempty"`
}

type Control struct {
	Op string `json:"op"`

	Target  string `json:"target,omitempty"`
	Session string `json:"session,omitempty"`

	Targets  []string `json:"targets,omitempty"`
	Sessions []string `json:"sessions,omitempty"`

	WinSize *WinSize `json:"winsize,omitempty"`

	OK    bool          `json:"ok,omitempty"`
	Error *ControlError `json:"error,omitempty"`
}
