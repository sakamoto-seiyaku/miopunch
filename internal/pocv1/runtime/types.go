package runtime

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocv1/presence"
)

type Stage string

const (
	StageNetwork       Stage = "Network"
	StageEnroll        Stage = "Enroll"
	StageDiscover      Stage = "Discover"
	StagePunch         Stage = "Punch"
	StageSecureSession Stage = "SecureSession"
	StageShell         Stage = "Shell"
)

type UserSummary struct {
	Text string `json:"text"`
}

type Evidence struct {
	Facts       []poc.Fact       `json:"facts"`
	Suggestions []poc.Suggestion `json:"suggestions"`
}

type PeerSession struct {
	PeerID             string `json:"peer_id"`
	Healthy            bool   `json:"healthy"`
	PathFamily         string `json:"path_family,omitempty"`
	Protocol           string `json:"protocol,omitempty"`
	LocalEndpoint      string `json:"local_endpoint,omitempty"`
	RemoteEndpoint     string `json:"remote_endpoint,omitempty"`
	LastActivityUnixMs int64  `json:"last_activity_unix_ms,omitempty"`
	PingGateSatisfied  bool   `json:"ping_gate_satisfied,omitempty"`
	ShellReadyUnixMs   int64  `json:"shell_ready_unix_ms,omitempty"`
	LastProvenUnixMs   int64  `json:"last_proven_unix_ms,omitempty"`
}

type ShellSession struct {
	ID              string `json:"id"`
	PeerID          string `json:"peer_id"`
	Target          string `json:"target,omitempty"`
	Session         string `json:"session,omitempty"`
	Status          string `json:"status"`
	CreatedAtUnixMs int64  `json:"created_at_unix_ms"`
	AttachedUnixMs  int64  `json:"attached_unix_ms,omitempty"`
	ClosedAtUnixMs  int64  `json:"closed_at_unix_ms,omitempty"`
}

type Snapshot struct {
	Stage         Stage                       `json:"stage"`
	ReasonCode    poc.ReasonCode              `json:"reason_code,omitempty"`
	Summary       UserSummary                 `json:"summary"`
	Evidence      Evidence                    `json:"evidence"`
	DiscoverView  presence.DiscoverProjection `json:"discover_view"`
	PeerSessions  []PeerSession               `json:"peer_sessions"`
	ShellSessions []ShellSession              `json:"shell_sessions"`
}

type Event struct {
	Kind     string   `json:"kind"`
	AtUnixMs int64    `json:"at_unix_ms"`
	Snapshot Snapshot `json:"snapshot"`
}

type ActionResult struct {
	Stage          Stage           `json:"stage"`
	ReasonCode     poc.ReasonCode  `json:"reason_code"`
	ExitCode       poc.ExitCode    `json:"exit_code"`
	Summary        UserSummary     `json:"summary"`
	Evidence       Evidence        `json:"evidence"`
	Snapshot       Snapshot        `json:"snapshot"`
	Lines          []string        `json:"lines,omitempty"`
	ReportMarkdown string          `json:"report_markdown,omitempty"`
	ShellSessionID string          `json:"shell_session_id,omitempty"`
	Data           json.RawMessage `json:"data,omitempty"`
}

type InitNetworkArgs struct {
	CreateNew bool   `json:"create_new,omitempty"`
	Confirm   string `json:"confirm,omitempty"`
}

type InviteArgs struct {
	Mode    string `json:"mode,omitempty"`
	MaxUses int    `json:"max_uses,omitempty"`
	Expires string `json:"expires,omitempty"`
}

type ApproveArgs struct {
	Code string `json:"code"`
}

type JoinArgs struct {
	Code string `json:"code"`
}

type PingArgs struct {
	PeerID     string `json:"peer_id"`
	P2PNetwork string `json:"p2p_network,omitempty"`
}

type ShellArgs struct {
	PeerID     string `json:"peer_id"`
	Target     string `json:"target,omitempty"`
	Session    string `json:"session,omitempty"`
	P2PNetwork string `json:"p2p_network,omitempty"`
}

type RevokeArgs struct {
	PeerID    string `json:"peer_id"`
	Dangerous bool   `json:"dangerous,omitempty"`
}

type problem struct {
	stage       Stage
	reasonCode  poc.ReasonCode
	exitCode    poc.ExitCode
	message     string
	facts       []poc.Fact
	suggestions []poc.Suggestion
}

func (p *problem) Error() string {
	if p == nil {
		return ""
	}
	if strings.TrimSpace(p.message) != "" {
		return p.message
	}
	return string(p.reasonCode)
}

func (p *problem) response() ErrorResponse {
	if p == nil {
		return ErrorResponse{}
	}
	return ErrorResponse{
		Stage:       string(p.stage),
		ReasonCode:  p.reasonCode,
		ExitCode:    p.exitCode,
		Message:     p.message,
		Facts:       cloneFacts(p.facts),
		Suggestions: cloneSuggestions(p.suggestions),
	}
}

func (p *problem) ErrorResponse() ErrorResponse {
	return p.response()
}

func newProblem(
	stage Stage,
	reason poc.ReasonCode,
	exitCode poc.ExitCode,
	message string,
	facts []poc.Fact,
	suggestions []poc.Suggestion,
) *problem {
	return &problem{
		stage:       stage,
		reasonCode:  reason,
		exitCode:    exitCode,
		message:     strings.TrimSpace(message),
		facts:       cloneFacts(facts),
		suggestions: cloneSuggestions(suggestions),
	}
}

func wrapProblem(stage Stage, reason poc.ReasonCode, exitCode poc.ExitCode, message string, err error, suggestions ...string) *problem {
	facts := []poc.Fact{}
	if strings.TrimSpace(message) != "" {
		facts = append(facts, poc.Fact{Message: message})
	}
	if err != nil {
		facts = append(facts, poc.Fact{Message: "error=" + err.Error()})
	}
	out := make([]poc.Suggestion, 0, len(suggestions))
	for _, suggestion := range suggestions {
		if strings.TrimSpace(suggestion) == "" {
			continue
		}
		out = append(out, poc.Suggestion{Message: suggestion})
	}
	return newProblem(stage, reason, exitCode, message, facts, out)
}

func cloneFacts(in []poc.Fact) []poc.Fact {
	out := make([]poc.Fact, 0, len(in))
	for _, fact := range in {
		out = append(out, fact)
	}
	return out
}

func cloneSuggestions(in []poc.Suggestion) []poc.Suggestion {
	out := make([]poc.Suggestion, 0, len(in))
	for _, suggestion := range in {
		out = append(out, suggestion)
	}
	return out
}

func parseArgs(raw json.RawMessage, out any) error {
	if len(raw) == 0 || string(raw) == "null" {
		return nil
	}
	if out == nil {
		return errors.New("nil args target")
	}
	if err := json.Unmarshal(raw, out); err != nil {
		return fmt.Errorf("decode args: %w", err)
	}
	return nil
}
