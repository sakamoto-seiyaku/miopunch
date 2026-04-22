package task

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

type InviteArgs struct {
	Mode    string `json:"mode,omitempty"`     // approve | auto
	MaxUses int    `json:"max_uses,omitempty"` // default 1
	Expires string `json:"expires,omitempty"`  // Go duration string (e.g. "15m")
}

type JoinArgs struct {
	Code string `json:"code"`
}

type ApproveArgs struct {
	Code string `json:"code"`
}

type PingArgs struct {
	PeerID string `json:"peer_id"`
}

type ShLSArgs struct {
	PeerID string `json:"peer_id"`
	Target string `json:"target,omitempty"`
}

type ShAttachArgs struct {
	PeerID  string `json:"peer_id"`
	Target  string `json:"target,omitempty"`
	Session string `json:"session,omitempty"`
}

type RevokeMemberArgs struct {
	PeerID    string `json:"peer_id"`
	Dangerous bool   `json:"dangerous,omitempty"`
}

func decodeArgs(raw json.RawMessage, out any) error {
	if out == nil {
		return errors.New("nil args target")
	}
	if len(raw) == 0 {
		// Treat empty as {} for forward compatibility.
		raw = []byte("{}")
	}
	if err := json.Unmarshal(raw, out); err != nil {
		return fmt.Errorf("invalid args: %w", err)
	}
	return nil
}

func (a InviteArgs) normalize() InviteArgs {
	a.Mode = strings.TrimSpace(a.Mode)
	a.Expires = strings.TrimSpace(a.Expires)
	return a
}

func (a JoinArgs) normalize() JoinArgs {
	a.Code = strings.TrimSpace(a.Code)
	return a
}

func (a ApproveArgs) normalize() ApproveArgs {
	a.Code = strings.TrimSpace(a.Code)
	return a
}

func (a PingArgs) normalize() PingArgs {
	a.PeerID = strings.TrimSpace(a.PeerID)
	return a
}

func (a ShLSArgs) normalize() ShLSArgs {
	a.PeerID = strings.TrimSpace(a.PeerID)
	a.Target = strings.TrimSpace(a.Target)
	return a
}

func (a ShAttachArgs) normalize() ShAttachArgs {
	a.PeerID = strings.TrimSpace(a.PeerID)
	a.Target = strings.TrimSpace(a.Target)
	a.Session = strings.TrimSpace(a.Session)
	return a
}

func (a RevokeMemberArgs) normalize() RevokeMemberArgs {
	a.PeerID = strings.TrimSpace(a.PeerID)
	return a
}
