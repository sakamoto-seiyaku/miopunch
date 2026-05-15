package task

import (
	"context"
	"encoding/json"
	"net"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/256dpi/gomqtt/broker"
	"github.com/256dpi/gomqtt/transport"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

func TestRunInviteTaskRejectsUnreachableBrokerBeforeEmittingCode(t *testing.T) {
	withBuiltinInviteBrokersForTest(t, nil)

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
	initAdminNetworkForTest(t, statePath)

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
	if !taskFactsContainSubstring(final, "broker_candidates="+broker) {
		t.Errorf("invite facts = %v, want broker_candidates fact containing %q", final.Facts, broker)
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

func TestRunInviteTaskDoesNotMixBuiltinBrokerWhenExplicitConfigExists(t *testing.T) {
	reachable := startTCPMQTTBroker(t)
	unreachable := unusedLocalTCPAddr(t)
	withBuiltinInviteBrokersForTest(t, []string{reachable})

	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			MQTTBroker:  unreachable,
			TopicPrefix: "miopunch/test",
			DataProto:   "quic",
			QUICCC:      "bbr",
		},
		Peers: map[string]pocstate.PeerConfig{},
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}
	initAdminNetworkForTest(t, statePath)

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
		t.Fatalf("invite ReasonCode = %q, want %q; facts=%v", final.ReasonCode, poc.ReasonCodeUnavailable, final.Facts)
	}
	if !taskFactsContainSubstring(final, "mqtt broker skipped: "+unreachable+":") {
		t.Errorf("invite facts = %v, want skipped broker diagnostic for %q", final.Facts, unreachable)
	}
	if taskFactsContainSubstring(final, "invite_brokers="+reachable) {
		t.Errorf("invite facts = %v, want no built-in invite_brokers fallback when explicit broker exists", final.Facts)
	}
	if taskFactsContainPrefix(final, "invite_code=") {
		t.Errorf("invite facts = %v, want no invite_code when explicit broker is unreachable", final.Facts)
	}
}

func TestRunInviteTaskUsesReachableBuiltinBrokerWhenExplicitConfigAbsent(t *testing.T) {
	reachable := startTCPMQTTBroker(t)
	unreachable := unusedLocalTCPAddr(t)
	withBuiltinInviteBrokersForTest(t, []string{unreachable, reachable})

	statePath := filepath.Join(t.TempDir(), "state.json")
	saveLocalStateForInviteTest(t, statePath)
	initAdminNetworkForTest(t, statePath)

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
	if final.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("invite ReasonCode = %q, want %q; facts=%v", final.ReasonCode, poc.ReasonCodeOK, final.Facts)
	}
	if !taskFactsContainSubstring(final, "mqtt broker skipped: "+unreachable+":") {
		t.Errorf("invite facts = %v, want skipped broker diagnostic for %q", final.Facts, unreachable)
	}
	if !taskFactsContainSubstring(final, "invite_brokers="+reachable) {
		t.Errorf("invite facts = %v, want invite_brokers fact containing %q", final.Facts, reachable)
	}

	st, err := pocstate.Load(statePath)
	if err != nil {
		t.Fatalf("pocstate.Load(%q) error = %v", statePath, err)
	}
	if got := st.Local.MQTTBrokerEndpoints(); len(got) != 1 || got[0] != reachable {
		t.Errorf("saved local.mqtt_broker = %v, want [%q]", got, reachable)
	}
}

func TestRunInviteTaskKeepsReachableHostnameBrokerInInviteCode(t *testing.T) {
	reachable := startTCPMQTTBroker(t)
	_, port, err := net.SplitHostPort(reachable)
	if err != nil {
		t.Fatalf("net.SplitHostPort(%q) error = %v", reachable, err)
	}
	hostnameBroker := net.JoinHostPort("localhost", port)
	withBuiltinInviteBrokersForTest(t, []string{hostnameBroker})

	statePath := filepath.Join(t.TempDir(), "state.json")
	saveLocalStateForInviteTest(t, statePath)
	initAdminNetworkForTest(t, statePath)

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
	if final.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("invite ReasonCode = %q, want %q; facts=%v", final.ReasonCode, poc.ReasonCodeOK, final.Facts)
	}
	if !taskFactsContainSubstring(final, "invite_brokers="+hostnameBroker) {
		t.Errorf("invite facts = %v, want invite_brokers fact containing %q", final.Facts, hostnameBroker)
	}

	code := inviteCodeFromTaskForTest(t, final)
	decoded, err := controlplane.DecodeInviteCodeV0(code)
	if err != nil {
		t.Fatalf("controlplane.DecodeInviteCodeV0(%q) error = %v", code, err)
	}
	if got := decoded.InviteBrokers; len(got) != 1 || got[0] != hostnameBroker {
		t.Errorf("DecodeInviteCodeV0(%q).InviteBrokers = %v, want [%q]", code, got, hostnameBroker)
	}

	st, err := pocstate.Load(statePath)
	if err != nil {
		t.Fatalf("pocstate.Load(%q) error = %v", statePath, err)
	}
	if got := st.Local.MQTTBrokerEndpoints(); len(got) != 1 || got[0] != hostnameBroker {
		t.Errorf("saved local.mqtt_broker = %v, want [%q]", got, hostnameBroker)
	}
}

func TestOpenMQTTMailboxesSkipsUnreachableBroker(t *testing.T) {
	reachable := startTCPMQTTBroker(t)
	unreachable := unusedLocalTCPAddr(t)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	mbs, failures, err := openMQTTMailboxes(ctx, []string{unreachable, reachable}, "miopunch-test")
	if err != nil {
		t.Fatalf("openMQTTMailboxes([%q %q]) error = %v", unreachable, reachable, err)
	}
	defer closeMQTTMailboxes(mbs)

	if len(mbs) != 1 {
		t.Fatalf("openMQTTMailboxes([%q %q]) opened %d mailboxes, want 1", unreachable, reachable, len(mbs))
	}
	if got := mbs[0].endpoint; got != reachable {
		t.Errorf("openMQTTMailboxes([%q %q]) endpoint = %q, want %q", unreachable, reachable, got, reachable)
	}
	if len(failures) != 1 || !strings.Contains(failures[0], unreachable+":") {
		t.Errorf("openMQTTMailboxes([%q %q]) failures = %v, want skipped %q", unreachable, reachable, failures, unreachable)
	}
}

func TestPublishMQTTAnyAllowsPartialFailure(t *testing.T) {
	reachable := startTCPMQTTBroker(t)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	mbs, _, err := openMQTTMailboxes(ctx, []string{reachable}, "miopunch-test")
	if err != nil {
		t.Fatalf("openMQTTMailboxes([%q]) error = %v", reachable, err)
	}
	defer closeMQTTMailboxes(mbs)

	bad := &mqttMailbox{endpoint: "bad.example:1883"}
	mbs = append([]*mqttMailbox{bad}, mbs...)
	if err := publishMQTTAny(ctx, mbs, "miopunch/test/topic", []byte("payload")); err != nil {
		t.Fatalf("publishMQTTAny(partial failure) error = %v, want nil", err)
	}
}

func TestSubscribeMQTTMailboxesAllowsPartialFailure(t *testing.T) {
	reachable := startTCPMQTTBroker(t)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	mbs, _, err := openMQTTMailboxes(ctx, []string{reachable}, "miopunch-test")
	if err != nil {
		t.Fatalf("openMQTTMailboxes([%q]) error = %v", reachable, err)
	}

	bad := &mqttMailbox{endpoint: "bad.example:1883"}
	subscribed, failures, err := subscribeMQTTMailboxes(ctx, append([]*mqttMailbox{bad}, mbs...), "miopunch/test/topic")
	if err != nil {
		t.Fatalf("subscribeMQTTMailboxes(partial failure) error = %v, want nil", err)
	}
	defer closeMQTTMailboxes(subscribed)

	if len(subscribed) != 1 {
		t.Fatalf("subscribeMQTTMailboxes(partial failure) subscribed %d mailboxes, want 1", len(subscribed))
	}
	if got := subscribed[0].endpoint; got != reachable {
		t.Errorf("subscribeMQTTMailboxes(partial failure) endpoint = %q, want %q", got, reachable)
	}
	if len(failures) != 1 || !strings.Contains(failures[0], "bad.example:1883:") {
		t.Errorf("subscribeMQTTMailboxes(partial failure) failures = %v, want bad.example:1883", failures)
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

func TestJoinApprovePersistEffectiveBrokerForPostJoinSignaling(t *testing.T) {
	effectiveBrokerA := startTCPMQTTBroker(t)
	effectiveBrokerB := startTCPMQTTBroker(t)
	withBuiltinInviteBrokersForTest(t, []string{effectiveBrokerA, effectiveBrokerB})

	approverStatePath := filepath.Join(t.TempDir(), "approver", "state.json")
	joinerStatePath := filepath.Join(t.TempDir(), "joiner", "state.json")

	saveLocalStateForInviteTest(t, approverStatePath)
	saveLocalStateForInviteTest(t, joinerStatePath)
	initAdminNetworkForTest(t, approverStatePath)

	approver := NewManagerWithStatePath(approverStatePath)
	t.Cleanup(approver.Close)
	joiner := NewManagerWithStatePath(joinerStatePath)
	t.Cleanup(joiner.Close)

	inviteRaw, err := json.Marshal(InviteArgs{Expires: "1m"})
	if err != nil {
		t.Fatalf("json.Marshal(InviteArgs) error = %v", err)
	}
	inviteCreated, err := approver.CreateAndRun(CreateRequest{Kind: "invite", Args: inviteRaw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(invite) error = %v", err)
	}
	inviteFinal := waitTaskDoneForTest(t, approver, inviteCreated.ID)
	if inviteFinal.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("invite ReasonCode = %q, want %q; facts=%v", inviteFinal.ReasonCode, poc.ReasonCodeOK, inviteFinal.Facts)
	}
	code := inviteCodeFromTaskForTest(t, inviteFinal)

	approveRaw, err := json.Marshal(ApproveArgs{Code: code})
	if err != nil {
		t.Fatalf("json.Marshal(ApproveArgs) error = %v", err)
	}
	approveCreated, err := approver.CreateAndRun(CreateRequest{Kind: "approve", Args: approveRaw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(approve) error = %v", err)
	}

	joinRaw, err := json.Marshal(JoinArgs{Code: code})
	if err != nil {
		t.Fatalf("json.Marshal(JoinArgs) error = %v", err)
	}
	joinCreated, err := joiner.CreateAndRun(CreateRequest{Kind: "join", Args: joinRaw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(join) error = %v", err)
	}

	joinFinal := waitTaskDoneForTest(t, joiner, joinCreated.ID)
	if joinFinal.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("join ReasonCode = %q, want %q; facts=%v", joinFinal.ReasonCode, poc.ReasonCodeOK, joinFinal.Facts)
	}
	approveFinal := waitTaskDoneForTest(t, approver, approveCreated.ID)
	if approveFinal.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("approve ReasonCode = %q, want %q; facts=%v", approveFinal.ReasonCode, poc.ReasonCodeOK, approveFinal.Facts)
	}

	joinerState, err := pocstate.Load(joinerStatePath)
	if err != nil {
		t.Fatalf("pocstate.Load(%q) error = %v", joinerStatePath, err)
	}
	if joinerState.Local == nil {
		t.Fatalf("joiner state local = nil, want saved local config")
	}
	if got := joinerState.Local.MQTTBrokerEndpoints(); !sameBrokerList(got, []string{effectiveBrokerA, effectiveBrokerB}) {
		t.Errorf("joiner local.mqtt_broker = %v, want [%q %q]", got, effectiveBrokerA, effectiveBrokerB)
	}

	approverState, err := pocstate.Load(approverStatePath)
	if err != nil {
		t.Fatalf("pocstate.Load(%q) error = %v", approverStatePath, err)
	}
	joinerPeerID := strings.TrimSpace(joinerState.Local.PeerID)
	if joinerPeerID == "" {
		t.Fatalf("joiner local.peer_id = empty, want joined identity")
	}
	cfg, ok := approverState.Peers[joinerPeerID]
	if !ok {
		t.Fatalf("approver peers[%q] missing; peers=%v", joinerPeerID, approverState.Peers)
	}
	if got := cfg.MQTTBrokerEndpoints(); !sameBrokerList(got, []string{effectiveBrokerA, effectiveBrokerB}) {
		t.Errorf("approver saved joiner mqtt_broker = %v, want [%q %q]", got, effectiveBrokerA, effectiveBrokerB)
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

func startTCPMQTTBroker(t *testing.T) string {
	t.Helper()

	server, err := transport.Launch("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("transport.Launch(tcp://127.0.0.1:0) error = %v", err)
	}
	backend := broker.NewMemoryBackend()
	engine := broker.NewEngine(backend)
	engine.Accept(server)

	t.Cleanup(func() {
		_ = server.Close()
		backend.Close(500 * time.Millisecond)
		engine.Close()
	})

	return server.Addr().String()
}

func saveLocalStateForInviteTest(t *testing.T, statePath string, brokers ...string) {
	t.Helper()

	local := &pocstate.LocalConfig{
		TopicPrefix: "miopunch/test",
		DataProto:   "quic",
		QUICCC:      "bbr",
	}
	local.SetMQTTBrokers(brokers)

	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local:  local,
		Peers:  map[string]pocstate.PeerConfig{},
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}
}

func initAdminNetworkForTest(t *testing.T, statePath string, brokers ...string) pocstate.Identity {
	t.Helper()

	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(%q) error = %v", stateDir, err)
	}
	netState, err := pocstate.EnsureNet(stateDir, brokers)
	if err != nil {
		t.Fatalf("pocstate.EnsureNet(%q) error = %v", stateDir, err)
	}
	if _, err := pocstate.EnsureGovernanceHeadSnapshot(stateDir, netState.NetID, selfID); err != nil {
		t.Fatalf("pocstate.EnsureGovernanceHeadSnapshot(%q) error = %v", stateDir, err)
	}
	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		t.Fatalf("pocstate.EnsureDecls(%q) error = %v", stateDir, err)
	}
	return selfID
}

func withBuiltinInviteBrokersForTest(t *testing.T, brokers []string) {
	t.Helper()

	orig := builtinInviteBrokers
	builtinInviteBrokers = func() []string {
		return append([]string(nil), brokers...)
	}
	t.Cleanup(func() {
		builtinInviteBrokers = orig
	})
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
	netState, err := pocstate.EnsureNet(stateDir, []string{broker})
	if err != nil {
		t.Fatalf("pocstate.EnsureNet(%q) error = %v", stateDir, err)
	}
	if _, err := pocstate.EnsureGovernanceHeadSnapshot(stateDir, netState.NetID, issuer); err != nil {
		t.Fatalf("pocstate.EnsureGovernanceHeadSnapshot(%q) error = %v", stateDir, err)
	}
	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		t.Fatalf("pocstate.EnsureDecls(%q) error = %v", stateDir, err)
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

func inviteCodeFromTaskForTest(t *testing.T, task Task) string {
	t.Helper()

	for _, fact := range task.Facts {
		if strings.TrimSpace(fact.TermID) != "invite_code" {
			continue
		}
		msg := strings.TrimSpace(fact.Message)
		return strings.TrimPrefix(msg, "invite_code=")
	}
	t.Fatalf("task facts = %v, want invite_code fact", task.Facts)
	return ""
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
