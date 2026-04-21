package task

import (
	"crypto/rand"
	"encoding/base32"
	"encoding/base64"
	"strings"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

var base32RawNoPad = base32.StdEncoding.WithPadding(base32.NoPadding)

func (m *Manager) runInviteTask(taskID string, rawArgs []byte) {
	var args InviteArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args = args.normalize()

	m.setStage(taskID, poc.StageSelfDiscovery, "prepare invite code")

	st, err := m.loadState()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "load state: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	if st.Local == nil {
		st.Local = &pocstate.LocalConfig{}
	}
	st.Local.NormalizeDefaults()

	// POC v0: invite is idempotent by default to avoid accidental rotation.
	if strings.TrimSpace(st.Local.PeerID) == "" {
		peerID, err := newPeerID()
		if err != nil {
			m.addFact(taskID, poc.Fact{Message: "new peer_id: " + err.Error()})
			m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
			m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
			return
		}
		st.Local.PeerID = peerID
	}
	if strings.TrimSpace(st.Local.ProxyName) == "" {
		st.Local.ProxyName = st.Local.PeerID
	}
	if strings.TrimSpace(st.Local.SecretKey) == "" {
		secretKey, err := newSecretKey()
		if err != nil {
			m.addFact(taskID, poc.Fact{Message: "new secret_key: " + err.Error()})
			m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
			m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
			return
		}
		st.Local.SecretKey = secretKey
	}

	st.EnsureLocalDefaults()

	jc := pocstate.JoinCode{
		PeerID:      st.Local.PeerID,
		ProxyName:   st.Local.ProxyName,
		SecretKey:   st.Local.SecretKey,
		MQTTBroker:  st.Local.MQTTBroker,
		TopicPrefix: st.Local.TopicPrefix,
		DataProto:   st.Local.DataProto,
		QUICCC:      st.Local.QUICCC,
	}
	code, err := pocstate.EncodeJoinCodeV0(jc)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "encode join code: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	// Persist before returning.
	if err := m.saveState(st); err != nil {
		m.addFact(taskID, poc.Fact{Message: "save state: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	m.addFact(taskID, poc.Fact{TermID: "peer_id", Message: "peer_id=" + st.Local.PeerID})
	m.addFact(taskID, poc.Fact{TermID: "invite_code", Message: "invite_code=" + code})
	m.addSuggestion(taskID, poc.Suggestion{Message: "on another machine: miopunch join <invite_code>"})
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}

func (m *Manager) runJoinTask(taskID string, rawArgs []byte) {
	var args JoinArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args = args.normalize()
	if args.Code == "" {
		m.addFact(taskID, poc.Fact{Message: "missing join code"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "use: miopunch join <code-or-url>"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	m.setStage(taskID, poc.StageControlPlaneReady, "import join code")

	code, err := pocstate.DecodeJoinCodeV0(args.Code)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid join code: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "verify the code and retry"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	st, err := m.loadState()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "load state: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	st.UpsertPeer(code.PeerID, code.ToPeerConfig())
	if err := m.saveState(st); err != nil {
		m.addFact(taskID, poc.Fact{Message: "save state: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	m.addFact(taskID, poc.Fact{TermID: "peer_id", Message: "peer_id=" + code.PeerID})
	m.addSuggestion(taskID, poc.Suggestion{Message: "list peers via: miopunch ls"})
	m.addSuggestion(taskID, poc.Suggestion{Message: "try: miopunch ping " + code.PeerID})
	m.addSuggestion(taskID, poc.Suggestion{Message: "try: miopunch sh " + code.PeerID})
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}

func newPeerID() (string, error) {
	// 6 bytes -> base32(raw,no-pad) -> 10 chars
	b := make([]byte, 6)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return "peer-" + strings.ToLower(base32RawNoPad.EncodeToString(b)), nil
}

func newSecretKey() (string, error) {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(b), nil
}
