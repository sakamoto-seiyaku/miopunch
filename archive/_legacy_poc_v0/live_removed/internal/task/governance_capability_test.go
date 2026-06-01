package task

import (
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

func TestRunInitNetworkTaskBootstrapsAdminNetwork(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	raw, err := json.Marshal(InitNetworkArgs{Mode: "bootstrap"})
	if err != nil {
		t.Fatalf("json.Marshal(InitNetworkArgs) error = %v", err)
	}
	created, err := m.CreateAndRun(CreateRequest{Kind: "init_network", Args: raw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(init_network) error = %v", err)
	}
	final := waitTaskDoneForTest(t, m, created.ID)
	if final.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("init_network ReasonCode = %q, want %q; facts=%v", final.ReasonCode, poc.ReasonCodeOK, final.Facts)
	}

	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(%q) error = %v", stateDir, err)
	}
	cap := m.localGovernanceCapability(stateDir, selfID)
	if cap.State != GovernanceStateAdminNetwork {
		t.Fatalf("localGovernanceCapability().State = %q, want %q", cap.State, GovernanceStateAdminNetwork)
	}
	if !cap.CanInvite || !cap.CanApprove {
		t.Fatalf("localGovernanceCapability() invite/approve = %v/%v, want true/true", cap.CanInvite, cap.CanApprove)
	}
}

func TestRunInitNetworkTaskCreateNewResetsNetworkState(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	saveLocalStateForInviteTest(t, statePath, "broker-a:1883")
	initAdminNetworkForTest(t, statePath, "broker-a:1883")
	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	oldNet, err := pocstate.LoadNet(stateDir)
	if err != nil {
		t.Fatalf("pocstate.LoadNet(%q) error = %v", stateDir, err)
	}
	if err := pocstate.SaveBootstrap(stateDir, pocstate.BootstrapFileV0{
		Recommendations: []pocstate.BootstrapPeerEvidenceV0{{PeerID: "peer-old", Bucket: "easy"}},
	}); err != nil {
		t.Fatalf("pocstate.SaveBootstrap(%q) error = %v", stateDir, err)
	}
	st, err := pocstate.Load(statePath)
	if err != nil {
		t.Fatalf("pocstate.Load(%q) error = %v", statePath, err)
	}
	st.Peers["peer-old"] = pocstate.PeerConfig{ProxyName: "old", SecretKey: "secret"}
	if err := pocstate.Save(statePath, st); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	raw, err := json.Marshal(InitNetworkArgs{Mode: "create_new", Confirm: "create-new-network"})
	if err != nil {
		t.Fatalf("json.Marshal(InitNetworkArgs) error = %v", err)
	}
	created, err := m.CreateAndRun(CreateRequest{Kind: "init_network", Args: raw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(init_network create_new) error = %v", err)
	}
	final := waitTaskDoneForTest(t, m, created.ID)
	if final.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("init_network create_new ReasonCode = %q, want %q; facts=%v", final.ReasonCode, poc.ReasonCodeOK, final.Facts)
	}

	newNet, err := pocstate.LoadNet(stateDir)
	if err != nil {
		t.Fatalf("pocstate.LoadNet(%q) after create_new error = %v", stateDir, err)
	}
	if newNet.NetID == oldNet.NetID {
		t.Fatalf("create_new net_id = %q, want different from old %q", newNet.NetID, oldNet.NetID)
	}
	st, err = pocstate.Load(statePath)
	if err != nil {
		t.Fatalf("pocstate.Load(%q) after create_new error = %v", statePath, err)
	}
	if len(st.Peers) != 0 {
		t.Fatalf("state peers after create_new = %v, want empty", st.Peers)
	}
	if _, err := pocstate.LoadBootstrap(stateDir); err == nil {
		t.Fatalf("pocstate.LoadBootstrap(%q) after create_new error = nil, want missing file", stateDir)
	}
}

func TestRunInviteTaskRejectsAutoMode(t *testing.T) {
	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	t.Cleanup(m.Close)

	raw, err := json.Marshal(InviteArgs{Mode: "auto"})
	if err != nil {
		t.Fatalf("json.Marshal(InviteArgs) error = %v", err)
	}
	created, err := m.CreateAndRun(CreateRequest{Kind: "invite", Args: raw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(invite auto) error = %v", err)
	}
	final := waitTaskDoneForTest(t, m, created.ID)
	if final.ReasonCode != poc.ReasonCodeNotImplemented {
		t.Fatalf("invite auto ReasonCode = %q, want %q; facts=%v", final.ReasonCode, poc.ReasonCodeNotImplemented, final.Facts)
	}
	if taskFactsContainPrefix(final, "invite_code=") {
		t.Fatalf("invite auto facts = %v, want no invite_code", final.Facts)
	}
}

func TestRunInviteTaskRejectsNonAdminLocalIdentity(t *testing.T) {
	broker := startTCPMQTTBroker(t)
	withBuiltinInviteBrokersForTest(t, []string{broker})
	statePath := filepath.Join(t.TempDir(), "state.json")
	saveLocalStateForInviteTest(t, statePath)
	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(%q) error = %v", stateDir, err)
	}
	adminID, err := pocstate.EnsureIdentity(t.TempDir())
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(admin) error = %v", err)
	}
	netState, err := pocstate.EnsureNet(stateDir, []string{broker})
	if err != nil {
		t.Fatalf("pocstate.EnsureNet(%q) error = %v", stateDir, err)
	}
	if _, err := pocstate.EnsureGovernanceHeadSnapshot(stateDir, netState.NetID, adminID); err != nil {
		t.Fatalf("pocstate.EnsureGovernanceHeadSnapshot(%q) error = %v", stateDir, err)
	}
	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		t.Fatalf("pocstate.EnsureDecls(%q) error = %v", stateDir, err)
	}

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)
	cap := m.localGovernanceCapability(stateDir, selfID)
	if cap.State != GovernanceStateForeignOrStaleNetwork {
		t.Fatalf("localGovernanceCapability().State = %q, want %q", cap.State, GovernanceStateForeignOrStaleNetwork)
	}

	raw, err := json.Marshal(InviteArgs{Expires: "1m"})
	if err != nil {
		t.Fatalf("json.Marshal(InviteArgs) error = %v", err)
	}
	created, err := m.CreateAndRun(CreateRequest{Kind: "invite", Args: raw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(invite) error = %v", err)
	}
	final := waitTaskDoneForTest(t, m, created.ID)
	if final.ReasonCode != poc.ReasonCodeForbidden {
		t.Fatalf("invite non-admin ReasonCode = %q, want %q; facts=%v", final.ReasonCode, poc.ReasonCodeForbidden, final.Facts)
	}
	if taskFactsContainPrefix(final, "invite_code=") {
		t.Fatalf("invite non-admin facts = %v, want no invite_code", final.Facts)
	}
}

func TestLocalGovernanceCapabilityClassifiesMemberNetwork(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	saveLocalStateForInviteTest(t, statePath)
	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	memberID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(%q) error = %v", stateDir, err)
	}
	adminID, err := pocstate.EnsureIdentity(t.TempDir())
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(admin) error = %v", err)
	}
	netState, err := pocstate.EnsureNet(stateDir, nil)
	if err != nil {
		t.Fatalf("pocstate.EnsureNet(%q) error = %v", stateDir, err)
	}
	if _, err := pocstate.EnsureGovernanceHeadSnapshot(stateDir, netState.NetID, adminID); err != nil {
		t.Fatalf("pocstate.EnsureGovernanceHeadSnapshot(%q) error = %v", stateDir, err)
	}
	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		t.Fatalf("pocstate.EnsureDecls(%q) error = %v", stateDir, err)
	}
	decl, err := pocstate.NewApproveMemberDeclV0(time.Now().UTC(), adminID, pocstate.ApproveMemberBodyV0{
		MemberPeerID:  memberID.PeerID,
		Ed25519PubB64: memberID.Ed25519PubB64(),
		X25519PubB64:  memberID.X25519PubB64(),
	})
	if err != nil {
		t.Fatalf("pocstate.NewApproveMemberDeclV0() error = %v", err)
	}
	if _, err := pocstate.UpdateDecls(stateDir, func(f *pocstate.DeclsFileV0) error {
		f.Decls = pocstate.AddDeclSetUnionV0(f.Decls, decl)
		return nil
	}); err != nil {
		t.Fatalf("pocstate.UpdateDecls(%q) error = %v", stateDir, err)
	}

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)
	cap := m.localGovernanceCapability(stateDir, memberID)
	if cap.State != GovernanceStateMemberNetwork {
		t.Fatalf("localGovernanceCapability().State = %q, want %q", cap.State, GovernanceStateMemberNetwork)
	}
	if cap.CanInvite || cap.CanApprove || !cap.CanCreateNewNetwork {
		t.Fatalf("localGovernanceCapability() invite/approve/create_new = %v/%v/%v, want false/false/true", cap.CanInvite, cap.CanApprove, cap.CanCreateNewNetwork)
	}
}
