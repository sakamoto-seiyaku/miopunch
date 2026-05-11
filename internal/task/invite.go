package task

import (
	"encoding/base64"
	"fmt"
	"net/url"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

const (
	defaultInviteMode   = controlplane.InviteModeApprove
	defaultInviteUses   = 1
	defaultInviteExpiry = 15 * time.Minute
)

var builtinInviteBrokers = pocstate.DefaultMQTTBrokers

func (m *Manager) runInviteTask(taskID string, rawArgs []byte) {
	var args InviteArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args = args.normalize()

	mode := defaultInviteMode
	if strings.TrimSpace(args.Mode) != "" {
		mode = controlplane.InviteMode(strings.TrimSpace(args.Mode))
	}

	maxUses := defaultInviteUses
	if args.MaxUses > 0 {
		maxUses = args.MaxUses
	}

	expires := defaultInviteExpiry
	if strings.TrimSpace(args.Expires) != "" {
		d, err := time.ParseDuration(args.Expires)
		if err != nil || d <= 0 {
			m.addFact(taskID, poc.Fact{Message: "invalid --expires duration"})
			m.addSuggestion(taskID, poc.Suggestion{Message: `use: --expires 15m`})
			m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
			return
		}
		expires = d
	}

	m.setStage(taskID, poc.StageSelfDiscovery, "prepare invite code")

	st, err := m.loadState()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "load state: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "state_dir: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure identity: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	if st.Local == nil {
		st.Local = &pocstate.LocalConfig{}
	}
	st.Local.NormalizeDefaults()

	// Bridge POC-06.5 governance identity into existing punching config.
	st.Local.PeerID = selfID.PeerID
	if strings.TrimSpace(st.Local.ProxyName) == "" {
		st.Local.ProxyName = selfID.PeerID
	}
	if strings.TrimSpace(st.Local.SecretKey) == "" {
		secretKey, _, err := newSecretKeyB64URLNoPad()
		if err != nil {
			m.addFact(taskID, poc.Fact{Message: "new secret_key: " + err.Error()})
			m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
			m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
			return
		}
		st.Local.SecretKey = secretKey
	}
	st.EnsureLocalDefaults()

	candidates := runtimeBrokerCandidates(st)
	if len(candidates) == 0 {
		m.addFact(taskID, poc.Fact{Message: "no mqtt broker candidates available"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "set local.mqtt_broker in state.json or restore built-in broker candidates"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	m.setStage(taskID, poc.StagePeerContact, "verify runtime brokers")
	effectiveBrokers, diagnostics, err := m.selectReachableRuntimeBrokers(candidates)
	for _, diagnostic := range diagnostics {
		if strings.TrimSpace(diagnostic) != "" {
			m.addFact(taskID, poc.Fact{Message: diagnostic})
		}
	}
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "broker_candidates=" + strings.Join(candidates, ",")})
		m.addFact(taskID, poc.Fact{Message: "mqtt connect failed: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "verify broker reachability and retry"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "set local.mqtt_broker to a reachable broker shared by both machines"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	m.setStage(taskID, poc.StagePeerContact, "verify invite brokers")
	inviteBrokers, inviteDiagnostics, err := m.selectReachableInviteBrokers(candidates, effectiveBrokers)
	for _, diagnostic := range inviteDiagnostics {
		if strings.TrimSpace(diagnostic) != "" {
			m.addFact(taskID, poc.Fact{Message: diagnostic})
		}
	}
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "broker_candidates=" + strings.Join(candidates, ",")})
		m.addFact(taskID, poc.Fact{Message: "mqtt connect failed: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "verify broker reachability and retry"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "set local.mqtt_broker to a reachable broker shared by both machines"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	m.addFact(taskID, poc.Fact{TermID: "invite_brokers", Message: "invite_brokers=" + strings.Join(inviteBrokers, ",")})
	m.addFact(taskID, poc.Fact{TermID: "brokers_effective", Message: "brokers_effective=" + strings.Join(effectiveBrokers, ",")})

	netState, err := pocstate.EnsureNet(stateDir, effectiveBrokers)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure net: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	if !sameBrokerList(netState.BrokersEffective, effectiveBrokers) {
		netState.BrokersEffective = append([]string(nil), effectiveBrokers...)
		if err := pocstate.SaveNet(stateDir, netState); err != nil {
			m.addFact(taskID, poc.Fact{Message: "save net: " + err.Error()})
			m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
			m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
			return
		}
	}

	if _, err := pocstate.EnsureGovernanceHeadSnapshot(stateDir, netState.NetID, selfID); err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure head snapshot: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure decls: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	inviteTopic, err := newRandomTopic()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "new invite_topic: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	inviteSecretB64, inviteSecret, err := newSecretKeyB64URLNoPad()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "new invite_secret: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	if len(inviteSecret) != 32 {
		m.addFact(taskID, poc.Fact{Message: "unexpected invite_secret length"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	expiresAtUnixMs := time.Now().UTC().Add(expires).UnixMilli()
	code, err := controlplane.EncodeInviteCodeV0(controlplane.InviteCodeV0{
		CodeType: inviteCodeTypeJoin,
		Version:  inviteCodeVersionV0,

		IssuerPeerID:        selfID.PeerID,
		IssuerEd25519PubB64: selfID.Ed25519PubB64(),
		IssuerX25519PubB64:  selfID.X25519PubB64(),

		InviteBrokers:   inviteBrokers,
		InviteTopic:     inviteTopic,
		InviteSecretB64: inviteSecretB64,
		Mode:            mode,
		MaxUses:         maxUses,
		ExpiresAtUnixMs: expiresAtUnixMs,
	})
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "encode invite code: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	// Persist state.json for punching (before returning code).
	st.Local.SetMQTTBrokers(effectiveBrokers)
	if err := m.saveState(st); err != nil {
		m.addFact(taskID, poc.Fact{Message: "save state: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	m.addFact(taskID, poc.Fact{TermID: "peer_id", Message: "peer_id=" + selfID.PeerID})
	m.addFact(taskID, poc.Fact{TermID: "net_id", Message: "net_id=" + netState.NetID})
	m.addFact(taskID, poc.Fact{TermID: "invite_code", Message: "invite_code=" + code})

	m.addSuggestion(taskID, poc.Suggestion{Message: "on this machine: miopunch approve <invite_code>"})
	m.addSuggestion(taskID, poc.Suggestion{Message: "on another machine: miopunch join <invite_code>"})
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}

func normalizeBrokerEndpoint(raw string) string {
	v := strings.TrimSpace(raw)
	if v == "" {
		return ""
	}
	if !strings.Contains(v, "://") {
		return v
	}

	u, err := url.Parse(v)
	if err != nil {
		return v
	}
	return strings.TrimSpace(u.Host)
}

func normalizeBrokerCandidates(candidates []string) []string {
	out := make([]string, 0, len(candidates))
	seen := make(map[string]struct{}, len(candidates))
	for _, raw := range candidates {
		ep := normalizeBrokerEndpoint(raw)
		if ep == "" {
			continue
		}
		if _, ok := seen[ep]; ok {
			continue
		}
		seen[ep] = struct{}{}
		out = append(out, ep)
	}
	return out
}

// These constants are duplicated from internal/controlplane to keep the invite
// task self-contained and avoid exporting internal details as public API.
const (
	inviteCodeTypeJoin  = "join"
	inviteCodeVersionV0 = 0
)

func decodeInviteSecretB64(value string) ([]byte, error) {
	b, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(value))
	if err != nil {
		return nil, err
	}
	if len(b) != 32 {
		return nil, fmt.Errorf("invalid invite_secret length: %d", len(b))
	}
	return b, nil
}
