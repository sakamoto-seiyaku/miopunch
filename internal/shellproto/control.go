package shellproto

import (
	"encoding/json"
	"time"
)

const (
	OpHello     = "hello"
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

	// POC-06.5: hello handshake (required before ping/sh_*).
	PeerID      string          `json:"peer_id,omitempty"`
	ApproveDecl json.RawMessage `json:"approve_decl,omitempty"`
	SigB64      string          `json:"sig_b64,omitempty"`

	Target  string `json:"target,omitempty"`
	Session string `json:"session,omitempty"`

	Targets  []string `json:"targets,omitempty"`
	Sessions []string `json:"sessions,omitempty"`

	WinSize *WinSize `json:"winsize,omitempty"`

	OK    bool          `json:"ok,omitempty"`
	Error *ControlError `json:"error,omitempty"`
}
