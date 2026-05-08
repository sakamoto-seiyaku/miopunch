package task

import (
	"encoding/json"
	"net"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

func TestRunInviteTaskRejectsUnreachableBrokerBeforeEmittingCode(t *testing.T) {
	broker := unusedLocalTCPAddr(t)
	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			MQTTBroker:  broker,
			TopicPrefix: "miopunch/test",
			DataProto:   "quic",
			QUICCC:      "bbr",
		},
		Peers: map[string]pocstate.PeerConfig{},
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	raw, err := json.Marshal(InviteArgs{Expires: "1m"})
	if err != nil {
		t.Fatalf("json.Marshal(InviteArgs) error = %v", err)
	}
	created, err := m.CreateAndRun(CreateRequest{Kind: "invite", Args: raw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(invite) error = %v", err)
	}

	final := waitTaskDoneForTest(t, m, created.ID)
	if final.ReasonCode != poc.ReasonCodeUnavailable {
		t.Errorf("invite ReasonCode = %q, want %q", final.ReasonCode, poc.ReasonCodeUnavailable)
	}
	if final.Stage != poc.StagePeerContact {
		t.Errorf("invite Stage = %q, want %q", final.Stage, poc.StagePeerContact)
	}
	if !taskFactsContainSubstring(final, "invite_brokers="+broker) {
		t.Errorf("invite facts = %v, want invite_brokers fact containing %q", final.Facts, broker)
	}
	if !taskFactsContainSubstring(final, "mqtt connect failed: "+broker+":") {
		t.Errorf("invite facts = %v, want mqtt failure fact containing broker %q", final.Facts, broker)
	}
	if taskFactsContainPrefix(final, "invite_code=") {
		t.Errorf("invite facts = %v, want no invite_code when broker is unreachable", final.Facts)
	}
	if !taskSuggestionsContainSubstring(final, "local.mqtt_broker") {
		t.Errorf("invite suggestions = %v, want broker configuration guidance", final.Suggestions)
	}
}

func TestRunJoinTaskReportsInviteBrokerOnConnectFailure(t *testing.T) {
	broker := unusedLocalTCPAddr(t)
	statePath := filepath.Join(t.TempDir(), "state.json")
	code := inviteCodeForTest(t, statePath, broker)

	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	t.Cleanup(m.Close)

	raw, err := json.Marshal(JoinArgs{Code: code})
	if err != nil {
		t.Fatalf("json.Marshal(JoinArgs) error = %v", err)
	}
	created, err := m.CreateAndRun(CreateRequest{Kind: "join", Args: raw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(join) error = %v", err)
	}

	final := waitTaskDoneForTest(t, m, created.ID)
	if final.ReasonCode != poc.ReasonCodeUnavailable {
		t.Errorf("join ReasonCode = %q, want %q", final.ReasonCode, poc.ReasonCodeUnavailable)
	}
	if final.Stage != poc.StagePeerContact {
		t.Errorf("join Stage = %q, want %q", final.Stage, poc.StagePeerContact)
	}
	if !taskFactsContainSubstring(final, "invite_brokers="+broker) {
		t.Errorf("join facts = %v, want invite_brokers fact containing %q", final.Facts, broker)
	}
	if !taskFactsContainSubstring(final, "mqtt connect failed: "+broker+":") {
		t.Errorf("join facts = %v, want mqtt failure fact containing broker %q", final.Facts, broker)
	}
}

func TestRunApproveTaskReportsInviteBrokerOnConnectFailure(t *testing.T) {
	broker := unusedLocalTCPAddr(t)
	statePath := filepath.Join(t.TempDir(), "state.json")
	code := inviteCodeForTest(t, statePath, broker)

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	raw, err := json.Marshal(ApproveArgs{Code: code})
	if err != nil {
		t.Fatalf("json.Marshal(ApproveArgs) error = %v", err)
	}
	created, err := m.CreateAndRun(CreateRequest{Kind: "approve", Args: raw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(approve) error = %v", err)
	}

	final := waitTaskDoneForTest(t, m, created.ID)
	if final.ReasonCode != poc.ReasonCodeUnavailable {
		t.Errorf("approve ReasonCode = %q, want %q", final.ReasonCode, poc.ReasonCodeUnavailable)
	}
	if final.Stage != poc.StagePeerContact {
		t.Errorf("approve Stage = %q, want %q", final.Stage, poc.StagePeerContact)
	}
	if !taskFactsContainSubstring(final, "invite_brokers="+broker) {
		t.Errorf("approve facts = %v, want invite_brokers fact containing %q", final.Facts, broker)
	}
	if !taskFactsContainSubstring(final, "mqtt connect failed: "+broker+":") {
		t.Errorf("approve facts = %v, want mqtt failure fact containing broker %q", final.Facts, broker)
	}
}

func unusedLocalTCPAddr(t *testing.T) string {
	t.Helper()

	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("net.Listen(tcp, 127.0.0.1:0) error = %v", err)
	}
	addr := ln.Addr().String()
	if err := ln.Close(); err != nil {
		t.Fatalf("Close unused local listener %q error = %v", addr, err)
	}
	return addr
}

func inviteCodeForTest(t *testing.T, statePath string, broker string) string {
	t.Helper()

	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	issuer, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(%q) error = %v", stateDir, err)
	}
	inviteTopic, err := newRandomTopic()
	if err != nil {
		t.Fatalf("newRandomTopic() error = %v", err)
	}
	inviteSecretB64, _, err := newSecretKeyB64URLNoPad()
	if err != nil {
		t.Fatalf("newSecretKeyB64URLNoPad() error = %v", err)
	}

	code, err := controlplane.EncodeInviteCodeV0(controlplane.InviteCodeV0{
		CodeType: inviteCodeTypeJoin,
		Version:  inviteCodeVersionV0,

		IssuerPeerID:        issuer.PeerID,
		IssuerEd25519PubB64: issuer.Ed25519PubB64(),
		IssuerX25519PubB64:  issuer.X25519PubB64(),

		InviteBrokers:   []string{broker},
		InviteTopic:     inviteTopic,
		InviteSecretB64: inviteSecretB64,
		Mode:            controlplane.InviteModeApprove,
		MaxUses:         1,
		ExpiresAtUnixMs: time.Now().UTC().Add(time.Minute).UnixMilli(),
	})
	if err != nil {
		t.Fatalf("controlplane.EncodeInviteCodeV0(%q) error = %v", broker, err)
	}
	return code
}

func taskFactsContainSubstring(task Task, needle string) bool {
	for _, fact := range task.Facts {
		if strings.Contains(fact.Message, needle) {
			return true
		}
	}
	return false
}

func taskFactsContainPrefix(task Task, prefix string) bool {
	for _, fact := range task.Facts {
		if strings.HasPrefix(strings.TrimSpace(fact.Message), prefix) {
			return true
		}
	}
	return false
}

func taskSuggestionsContainSubstring(task Task, needle string) bool {
	for _, suggestion := range task.Suggestions {
		if strings.Contains(suggestion.Message, needle) {
			return true
		}
	}
	return false
}
