package task

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func TestBuildLocalPresenceEvidenceSignsStateHead(t *testing.T) {
	stateDir := t.TempDir()
	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity() error = %v", err)
	}
	now := time.Unix(1_700_000_000, 0).UTC()
	stateHead := TopologyStateHead{
		GovernanceHeadB64: "gov-head",
		DeclsHeadB64:      "decls-head",
	}
	self := TopologySelf{
		PeerID: selfID.PeerID,
		V4Hint: "easy",
		V6Hint: "direct",
	}

	got, err := buildLocalPresenceEvidence(now, selfID, stateHead, self)
	if err != nil {
		t.Fatalf("buildLocalPresenceEvidence() error = %v", err)
	}
	if !got.Signed {
		t.Fatal("buildLocalPresenceEvidence().Signed = false, want true")
	}
	if got.StateHead != stateHead {
		t.Fatalf("buildLocalPresenceEvidence().StateHead = %#v, want %#v", got.StateHead, stateHead)
	}

	body := struct {
		PeerID    string            `json:"peer_id"`
		StateHead TopologyStateHead `json:"state_head"`
		V4Hint    string            `json:"v4_hint,omitempty"`
		V6Hint    string            `json:"v6_hint,omitempty"`
	}{
		PeerID:    selfID.PeerID,
		StateHead: stateHead,
		V4Hint:    "easy",
		V6Hint:    "direct",
	}
	bodyJSON, err := json.Marshal(body)
	if err != nil {
		t.Fatalf("json.Marshal(body) error = %v", err)
	}
	msg := controlplane.Message{
		ProtoVersion: controlplane.ProtoVersionV0,
		Route: controlplane.Route{
			DstPeerID:       selfID.PeerID,
			MsgID:           got.MessageID,
			HopLimit:        0,
			CreatedAtUnixMs: now.UnixMilli(),
			ExpiresAtUnixMs: now.Add(2 * time.Minute).UnixMilli(),
		},
		Signed: controlplane.Signed{
			SenderPeerID: selfID.PeerID,
			Kind:         "presence",
			Body:         bodyJSON,
			SigB64:       got.SigB64,
		},
	}
	if err := controlplane.VerifyV0(selfID.Ed25519Pub, msg); err != nil {
		t.Fatalf("controlplane.VerifyV0(presence) error = %v", err)
	}
}

func TestTopologySnapshotLoadsBootstrapEvidence(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	stateDir := filepath.Dir(statePath)
	if _, err := pocstate.EnsureIdentity(stateDir); err != nil {
		t.Fatalf("pocstate.EnsureIdentity() error = %v", err)
	}
	if err := pocstate.SaveBootstrap(stateDir, pocstate.BootstrapFileV0{
		Recommendations: []pocstate.BootstrapPeerEvidenceV0{
			{PeerID: "peer-a", Bucket: "direct", Reason: "approver_admin"},
			{PeerID: "peer-b", Bucket: "easy", Reason: "known_joined_seed"},
		},
	}); err != nil {
		t.Fatalf("pocstate.SaveBootstrap() error = %v", err)
	}

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	got, err := m.TopologySnapshot()
	if err != nil {
		t.Fatalf("TopologySnapshot() error = %v", err)
	}
	if len(got.Bootstrap.Recommendations) != 2 {
		t.Fatalf("TopologySnapshot().Bootstrap.Recommendations length = %d, want 2", len(got.Bootstrap.Recommendations))
	}
	if got.Bootstrap.Recommendations[0].PeerID != "peer-a" {
		t.Errorf("TopologySnapshot().Bootstrap.Recommendations[0].PeerID = %q, want peer-a", got.Bootstrap.Recommendations[0].PeerID)
	}
	if got.Presence.Local == nil || !got.Presence.Local.Signed {
		t.Fatalf("TopologySnapshot().Presence.Local signed = %#v, want signed local presence", got.Presence.Local)
	}
}

func TestTopologySnapshotIncludesKnownSeedPeersMissingFromDecls(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	stateDir := filepath.Dir(statePath)
	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(self) error = %v", err)
	}
	issuerID, err := pocstate.EnsureIdentity(t.TempDir())
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(issuer) error = %v", err)
	}

	netID, err := pocstate.NetIDFromSecret(bytes.Repeat([]byte{2}, 32))
	if err != nil {
		t.Fatalf("pocstate.NetIDFromSecret() error = %v", err)
	}
	if err := pocstate.SaveNet(stateDir, pocstate.Net{NetSecret: bytes.Repeat([]byte{2}, 32)}); err != nil {
		t.Fatalf("pocstate.SaveNet() error = %v", err)
	}
	if _, err := pocstate.EnsureGovernanceHeadSnapshot(stateDir, netID, issuerID); err != nil {
		t.Fatalf("pocstate.EnsureGovernanceHeadSnapshot() error = %v", err)
	}
	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		t.Fatalf("pocstate.EnsureDecls() error = %v", err)
	}
	if _, err := pocstate.UpdateDecls(stateDir, func(f *pocstate.DeclsFileV0) error {
		f.Decls = append(f.Decls, mustApproveDecl(t, issuerID, selfID, "unknown"))
		return nil
	}); err != nil {
		t.Fatalf("pocstate.UpdateDecls() error = %v", err)
	}
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			PeerID:      selfID.PeerID,
			ProxyName:   selfID.PeerID,
			SecretKey:   "self-secret",
			MQTTBroker:  "broker:1883",
			TopicPrefix: "miopunch/test",
		},
		Peers: map[string]pocstate.PeerConfig{
			issuerID.PeerID: testPeerConfig(),
		},
	}); err != nil {
		t.Fatalf("pocstate.Save() error = %v", err)
	}

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	got, err := m.TopologySnapshot()
	if err != nil {
		t.Fatalf("TopologySnapshot() error = %v", err)
	}
	if !topologyMembersContain(got.Members, issuerID.PeerID) {
		t.Fatalf("TopologySnapshot().Members = %#v, want known issuer seed peer %q", got.Members, issuerID.PeerID)
	}
	if !topologyMembersContain(got.Members, selfID.PeerID) {
		t.Fatalf("TopologySnapshot().Members = %#v, want approved self peer %q", got.Members, selfID.PeerID)
	}
}

func TestTopologyPortmapEvidenceFromEvents(t *testing.T) {
	raw := `{"name":"gather.portmap.snapshot","kvs":{"included":true,"direct_v4":1,"methods_done":2}}` + "\n"

	got := topologyPortmapEvidenceFromEvents(raw)
	if got == nil {
		t.Fatal("topologyPortmapEvidenceFromEvents() = nil, want evidence")
	}
	if !got.UDPIncluded {
		t.Error("topologyPortmapEvidenceFromEvents().UDPIncluded = false, want true")
	}
	if got.UDPDirectV4 != 1 {
		t.Errorf("topologyPortmapEvidenceFromEvents().UDPDirectV4 = %d, want 1", got.UDPDirectV4)
	}
	if got.MethodsDone != 2 {
		t.Errorf("topologyPortmapEvidenceFromEvents().MethodsDone = %d, want 2", got.MethodsDone)
	}
}

func TestTargetNeighborKForTwelveMembers(t *testing.T) {
	got := targetNeighborK(12)
	if got != 3 {
		t.Fatalf("targetNeighborK(12) = %d, want 3", got)
	}
}

func TestSelectTopologyNeighborsUsesBucketRotationAndAvoidsAdminOnly(t *testing.T) {
	members := []TopologyMember{
		{PeerID: "self", Role: "member", V4Hint: "hard1", V6Hint: "none"},
		{PeerID: "admin", Role: "admin", V4Hint: "direct", V6Hint: "none"},
		{PeerID: "easy-a", Role: "member", V4Hint: "easy", V6Hint: "none"},
		{PeerID: "easy-b", Role: "member", V4Hint: "easy", V6Hint: "none"},
		{PeerID: "hard-a", Role: "member", V4Hint: "hard1", V6Hint: "none"},
	}
	st := pocstate.State{Peers: map[string]pocstate.PeerConfig{
		"admin":  testPeerConfig(),
		"easy-a": testPeerConfig(),
		"easy-b": testPeerConfig(),
		"hard-a": testPeerConfig(),
	}}

	got := selectTopologyNeighbors("self", members, TopologyBootstrap{}, st, 3)
	if len(got) != 3 {
		t.Fatalf("selectTopologyNeighbors(self, members, st, 3) length = %d, want 3: %#v", len(got), got)
	}

	nonAdmin := 0
	easy := 0
	for _, selected := range got {
		if selected.Role != "admin" && selected.Role != "owner" {
			nonAdmin++
		}
		if selected.Bucket == "easy" {
			easy++
		}
		if !selected.Dialable {
			t.Errorf("selectTopologyNeighbors selected %q Dialable=false, want true", selected.PeerID)
		}
	}
	if nonAdmin == 0 {
		t.Fatalf("selectTopologyNeighbors selected %#v, want at least one non-admin", got)
	}
	if easy == 0 {
		t.Fatalf("selectTopologyNeighbors selected %#v, want easy bucket carry candidate", got)
	}
}

func TestSelectTopologyNeighborsPrefersDialableCandidates(t *testing.T) {
	members := []TopologyMember{
		{PeerID: "self", Role: "member", V4Hint: "direct", V6Hint: "direct"},
		{PeerID: "direct-a", Role: "member", V4Hint: "direct", V6Hint: "none"},
		{PeerID: "direct-b", Role: "member", V4Hint: "hard1", V6Hint: "direct"},
		{PeerID: "easy-a", Role: "member", V4Hint: "easy", V6Hint: "none"},
	}
	st := pocstate.State{Peers: map[string]pocstate.PeerConfig{
		"easy-a": testPeerConfig(),
	}}

	got := selectTopologyNeighbors("self", members, TopologyBootstrap{}, st, 3)
	if len(got) != 3 {
		t.Fatalf("selectTopologyNeighbors(self, members, st, 3) length = %d, want 3: %#v", len(got), got)
	}
	if got[0].PeerID != "easy-a" || !got[0].Dialable {
		t.Fatalf("selectTopologyNeighbors(...)[0] = %#v, want dialable easy-a first", got[0])
	}
}

func TestSelectTopologyNeighborsIncludesDialableBootstrapCandidates(t *testing.T) {
	members := []TopologyMember{
		{PeerID: "self", Role: "member", V4Hint: "easy", V6Hint: "none"},
		{PeerID: "seed", Role: "member", V4Hint: "direct", V6Hint: "direct"},
		{PeerID: "direct-a", Role: "member", V4Hint: "direct", V6Hint: "none"},
		{PeerID: "direct-b", Role: "member", V4Hint: "direct", V6Hint: "none"},
	}
	bootstrap := TopologyBootstrap{
		Recommendations: []TopologyPeerEvidence{
			{PeerID: "admin", Bucket: "direct", Reason: "approver_admin"},
			{PeerID: "seed", Bucket: "direct", Reason: "known_joined_seed"},
		},
	}
	st := pocstate.State{Peers: map[string]pocstate.PeerConfig{
		"admin": testPeerConfig(),
		"seed":  testPeerConfig(),
	}}

	got := selectTopologyNeighbors("self", members, bootstrap, st, 3)
	if len(got) != 3 {
		t.Fatalf("selectTopologyNeighbors(self, members, bootstrap, st, 3) length = %d, want 3: %#v", len(got), got)
	}

	if !topologySelectionsContain(got[:2], "admin", true) {
		t.Fatalf("selectTopologyNeighbors(...)[0:2] = %#v, want dialable bootstrap admin", got[:2])
	}
	if !topologySelectionsContain(got[:2], "seed", true) {
		t.Fatalf("selectTopologyNeighbors(...)[0:2] = %#v, want dialable member seed", got[:2])
	}
}

func TestTopologyNeighborFailuresDescribeHardCarryStopCondition(t *testing.T) {
	members := topologyMembersByPeerID([]TopologyMember{
		{PeerID: "hard-node", V4Hint: "hard2", V6Hint: "none"},
	})
	attempts := []TopologyAttempt{{
		PeerID:        "hard-node",
		Outcome:       "fail",
		Stage:         "PeerContact",
		ReasonCode:    "UNAVAILABLE",
		StopCondition: "dial_failed",
	}}

	got := topologyNeighborFailures(attempts, members)
	if len(got) != 1 {
		t.Fatalf("topologyNeighborFailures(attempts, members) length = %d, want 1: %#v", len(got), got)
	}
	if got[0].Bucket != "hard2" {
		t.Errorf("topologyNeighborFailures(...)[0].Bucket = %q, want hard2", got[0].Bucket)
	}
	if got[0].StopCondition != "dial_failed" {
		t.Errorf("topologyNeighborFailures(...)[0].StopCondition = %q, want dial_failed", got[0].StopCondition)
	}
	if got[0].RetryBudget != 1 {
		t.Errorf("topologyNeighborFailures(...)[0].RetryBudget = %d, want 1", got[0].RetryBudget)
	}
}

func TestTopologyNeighborReplacementSelectsNewCandidate(t *testing.T) {
	selected := []TopologyNeighborSelection{
		{PeerID: "old", Bucket: "direct", Dialable: true},
		{PeerID: "new", Bucket: "easy", Dialable: true},
	}
	active := []TopologyNeighborEdge{{PeerID: "other", Healthy: true}}
	unhealthy := []TopologyNeighborHealth{{PeerID: "old", CloseReason: "transport_fatal"}}

	got := topologyNeighborReplacements(selected, active, unhealthy)
	if len(got) != 1 {
		t.Fatalf("topologyNeighborReplacements(...) length = %d, want 1: %#v", len(got), got)
	}
	if got[0].OldPeerID != "old" || got[0].NewPeerID != "new" {
		t.Fatalf("topologyNeighborReplacements(...)[0] = %#v, want old->new", got[0])
	}
}

func TestRunMaintainNeighborsPingsSelectedPeers(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	stateDir := filepath.Dir(statePath)
	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(self) error = %v", err)
	}
	peerA, err := pocstate.EnsureIdentity(t.TempDir())
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(peerA) error = %v", err)
	}
	peerB, err := pocstate.EnsureIdentity(t.TempDir())
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(peerB) error = %v", err)
	}

	netID, err := pocstate.NetIDFromSecret(bytes.Repeat([]byte{1}, 32))
	if err != nil {
		t.Fatalf("pocstate.NetIDFromSecret() error = %v", err)
	}
	if err := pocstate.SaveNet(stateDir, pocstate.Net{NetSecret: bytes.Repeat([]byte{1}, 32)}); err != nil {
		t.Fatalf("pocstate.SaveNet() error = %v", err)
	}
	if _, err := pocstate.EnsureGovernanceHeadSnapshot(stateDir, netID, selfID); err != nil {
		t.Fatalf("pocstate.EnsureGovernanceHeadSnapshot() error = %v", err)
	}
	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		t.Fatalf("pocstate.EnsureDecls() error = %v", err)
	}
	decls := []pocstate.DeclV0{
		mustApproveDecl(t, selfID, selfID, "direct"),
		mustApproveDecl(t, selfID, peerA, "easy"),
		mustApproveDecl(t, selfID, peerB, "easy"),
	}
	if _, err := pocstate.UpdateDecls(stateDir, func(f *pocstate.DeclsFileV0) error {
		f.Decls = append(f.Decls, decls...)
		return nil
	}); err != nil {
		t.Fatalf("pocstate.UpdateDecls() error = %v", err)
	}
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			PeerID:      selfID.PeerID,
			ProxyName:   "self",
			SecretKey:   "self-secret",
			MQTTBroker:  "broker:1883",
			TopicPrefix: "miopunch/test",
		},
		Peers: map[string]pocstate.PeerConfig{
			peerA.PeerID: testPeerConfig(),
			peerB.PeerID: testPeerConfig(),
		},
	}); err != nil {
		t.Fatalf("pocstate.Save() error = %v", err)
	}

	calls := make(map[string]int)
	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)
	m.SetDialPeerStreamHook(func(ctx context.Context, taskID string, peerID string, cfg pocstate.PeerConfig) (io.ReadWriteCloser, error) {
		calls[peerID]++
		clientConn, serverConn := net.Pipe()
		go servePingPeerForTest(serverConn)
		return clientConn, nil
	})

	raw, err := json.Marshal(MaintainNeighborsArgs{})
	if err != nil {
		t.Fatalf("json.Marshal(MaintainNeighborsArgs{}) error = %v", err)
	}
	created, err := m.CreateAndRun(CreateRequest{Kind: "maintain_neighbors", Args: raw})
	if err != nil {
		t.Fatalf("CreateAndRun(maintain_neighbors) error = %v", err)
	}
	final := waitTaskDoneForTest(t, m, created.ID)
	if final.ExitCode != poc.ExitCodeOK {
		t.Fatalf("maintain_neighbors ExitCode = %d, want %d; facts=%#v", final.ExitCode, poc.ExitCodeOK, final.Facts)
	}
	if calls[peerA.PeerID] != 1 {
		t.Errorf("maintain_neighbors calls[%q] = %d, want 1", peerA.PeerID, calls[peerA.PeerID])
	}
	if calls[peerB.PeerID] != 1 {
		t.Errorf("maintain_neighbors calls[%q] = %d, want 1", peerB.PeerID, calls[peerB.PeerID])
	}
	if !taskFactsContain(final, "maintain_neighbors_succeeded=2") {
		t.Fatalf("maintain_neighbors facts = %#v, want maintain_neighbors_succeeded=2", final.Facts)
	}
}

func mustApproveDecl(t *testing.T, issuer pocstate.Identity, member pocstate.Identity, v4Hint string) pocstate.DeclV0 {
	t.Helper()
	decl, err := pocstate.NewApproveMemberDeclV0(time.Now().UTC(), issuer, pocstate.ApproveMemberBodyV0{
		MemberPeerID:  member.PeerID,
		Ed25519PubB64: member.Ed25519PubB64(),
		X25519PubB64:  member.X25519PubB64(),
		V4Hint:        v4Hint,
		V6Hint:        "none",
	})
	if err != nil {
		t.Fatalf("pocstate.NewApproveMemberDeclV0(%q) error = %v", member.PeerID, err)
	}
	return decl
}

func servePingPeerForTest(conn io.ReadWriteCloser) {
	defer conn.Close()
	var hello shellproto.Control
	if err := shellproto.ReadJSON(conn, &hello); err != nil {
		return
	}
	_ = shellproto.WriteJSON(conn, shellproto.Control{Op: shellproto.OpHello, OK: true})
	var req shellproto.Control
	if err := shellproto.ReadJSON(conn, &req); err != nil {
		return
	}
	if strings.TrimSpace(req.Op) != shellproto.OpPing {
		_ = shellproto.WriteJSON(conn, shellproto.Control{
			Op: shellproto.OpPing,
			OK: false,
			Error: &shellproto.ControlError{
				ReasonCode: shellproto.ReasonHelloRequired,
				Message:    "unexpected op",
			},
		})
		return
	}
	_ = shellproto.WriteJSON(conn, shellproto.Control{Op: shellproto.OpPing, OK: true})
}

func waitTaskDoneForTest(t *testing.T, m *Manager, taskID string) Task {
	t.Helper()
	deadline := time.After(5 * time.Second)
	ticker := time.NewTicker(10 * time.Millisecond)
	defer ticker.Stop()
	for {
		if got, ok := m.Get(taskID); ok && got.Status == StatusDone {
			return got
		}
		select {
		case <-deadline:
			t.Fatalf("Manager.Get(%q) did not reach done", taskID)
		case <-ticker.C:
		}
	}
}

func taskFactsContain(task Task, needle string) bool {
	for _, fact := range task.Facts {
		if strings.TrimSpace(fact.Message) == needle {
			return true
		}
	}
	return false
}

func topologySelectionsContain(selections []TopologyNeighborSelection, peerID string, dialable bool) bool {
	for _, selection := range selections {
		if selection.PeerID == peerID && selection.Dialable == dialable {
			return true
		}
	}
	return false
}

func topologyMembersContain(members []TopologyMember, peerID string) bool {
	for _, member := range members {
		if member.PeerID == peerID {
			return true
		}
	}
	return false
}

func testPeerConfig() pocstate.PeerConfig {
	return pocstate.PeerConfig{
		ProxyName:   "peer",
		SecretKey:   "secret",
		MQTTBroker:  "broker:1883",
		TopicPrefix: "miopunch/test",
	}
}
