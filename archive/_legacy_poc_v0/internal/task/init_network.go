package task

import (
	"crypto/rand"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

const (
	initNetworkModeBootstrap = "bootstrap"
	initNetworkModeCreateNew = "create_new"
	initNetworkConfirmNew    = "create-new-network"
)

func (m *Manager) runInitNetworkTask(taskID string, rawArgs []byte) {
	var args InitNetworkArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args = args.normalize()
	if args.Mode == "" {
		args.Mode = initNetworkModeBootstrap
	}
	if args.Mode != initNetworkModeBootstrap && args.Mode != initNetworkModeCreateNew {
		m.addFact(taskID, poc.Fact{Message: "invalid init_network mode: " + args.Mode})
		m.addSuggestion(taskID, poc.Suggestion{Message: "use mode=bootstrap or mode=create_new"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	if args.Mode == initNetworkModeCreateNew && args.Confirm != initNetworkConfirmNew {
		m.addFact(taskID, poc.Fact{Message: "missing create-new-network confirmation"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "set confirm=create-new-network"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	m.setStage(taskID, poc.StageSelfDiscovery, "initialize local network")

	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "state_dir: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure identity: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	cap := m.localGovernanceCapability(stateDir, selfID)
	if args.Mode == initNetworkModeBootstrap && cap.State != GovernanceStateNoNetwork {
		m.addGovernanceCapabilityFacts(taskID, cap)
		m.addFact(taskID, poc.Fact{Message: "local network already exists"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "use init-network --new --confirm create-new-network to create a distinct new network"})
		m.done(taskID, poc.ReasonCodeConflict, poc.ExitCodeConflict)
		return
	}

	oldNetID := strings.TrimSpace(cap.NetID)
	if args.Mode == initNetworkModeCreateNew {
		if err := m.resetLocalNetworkState(stateDir); err != nil {
			m.addFact(taskID, poc.Fact{Message: "reset local network state: " + err.Error()})
			m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
			return
		}
	}

	netState, head, err := m.createLocalGenesisNetwork(stateDir, selfID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "create local network: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	if err := m.prepareLocalConfigForIdentity(selfID, netState.BrokersEffective, true); err != nil {
		m.addFact(taskID, poc.Fact{Message: "save local config: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	m.addFact(taskID, poc.Fact{TermID: "peer_id", Message: "peer_id=" + selfID.PeerID})
	m.addFact(taskID, poc.Fact{TermID: "net_id", Message: "net_id=" + netState.NetID})
	m.addFact(taskID, poc.Fact{TermID: "governance_head_b64", Message: "governance_head_b64=" + head.HashB64})
	if oldNetID != "" && oldNetID != netState.NetID {
		m.addFact(taskID, poc.Fact{TermID: "old_net_id", Message: "old_net_id=" + oldNetID})
	}
	m.publishDesktopConfigAndTopologyChange()
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}

func (m *Manager) createLocalGenesisNetwork(stateDir string, selfID pocstate.Identity) (pocstate.Net, pocstate.GovernanceHeadSnapshotV1, error) {
	netSecret := make([]byte, 32)
	if _, err := rand.Read(netSecret); err != nil {
		return pocstate.Net{}, pocstate.GovernanceHeadSnapshotV1{}, fmt.Errorf("read net secret: %w", err)
	}
	netID, err := pocstate.NetIDFromSecret(netSecret)
	if err != nil {
		return pocstate.Net{}, pocstate.GovernanceHeadSnapshotV1{}, err
	}
	netState := pocstate.Net{
		NetID:     netID,
		NetSecret: netSecret,
	}
	if err := pocstate.SaveNet(stateDir, netState); err != nil {
		return pocstate.Net{}, pocstate.GovernanceHeadSnapshotV1{}, fmt.Errorf("save net: %w", err)
	}
	head, err := pocstate.EnsureGovernanceHeadSnapshot(stateDir, netState.NetID, selfID)
	if err != nil {
		return pocstate.Net{}, pocstate.GovernanceHeadSnapshotV1{}, fmt.Errorf("ensure governance head: %w", err)
	}
	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		return pocstate.Net{}, pocstate.GovernanceHeadSnapshotV1{}, fmt.Errorf("ensure decls: %w", err)
	}
	return netState, head, nil
}

func (m *Manager) resetLocalNetworkState(stateDir string) error {
	for _, rel := range []string{
		"net.json",
		filepath.Join("governance", "head_snapshot.json"),
		"decls.json",
		"bootstrap",
		"invites",
	} {
		path := filepath.Join(stateDir, rel)
		if err := os.RemoveAll(path); err != nil && !errors.Is(err, os.ErrNotExist) {
			return fmt.Errorf("remove %s: %w", rel, err)
		}
	}

	st, err := m.loadState()
	if err != nil {
		return fmt.Errorf("load state: %w", err)
	}
	st.Peers = map[string]pocstate.PeerConfig{}
	if st.Local != nil {
		st.Local.NormalizeDefaults()
	}
	if err := m.saveState(st); err != nil {
		return fmt.Errorf("save state: %w", err)
	}
	return nil
}

func (m *Manager) prepareLocalConfigForIdentity(selfID pocstate.Identity, brokers []string, preserveBrokers bool) error {
	st, err := m.loadState()
	if err != nil {
		return err
	}
	if st.Local == nil {
		st.Local = &pocstate.LocalConfig{}
	}
	existingBrokers := st.Local.MQTTBrokerEndpoints()
	st.Local.NormalizeDefaults()
	st.Local.PeerID = selfID.PeerID
	if strings.TrimSpace(st.Local.ProxyName) == "" {
		st.Local.ProxyName = selfID.PeerID
	}
	if strings.TrimSpace(st.Local.SecretKey) == "" {
		secretKey, _, err := newSecretKeyB64URLNoPad()
		if err != nil {
			return fmt.Errorf("new secret_key: %w", err)
		}
		st.Local.SecretKey = secretKey
	}
	switch {
	case len(brokers) > 0:
		st.Local.SetMQTTBrokers(brokers)
	case preserveBrokers && len(existingBrokers) > 0:
		st.Local.SetMQTTBrokers(existingBrokers)
	}
	st.EnsureLocalDefaults()
	return m.saveState(st)
}

func (m *Manager) requireAdminCapability(taskID string, stateDir string, selfID pocstate.Identity) (localGovernanceCapability, bool) {
	cap := m.localGovernanceCapability(stateDir, selfID)
	if cap.State == GovernanceStateAdminNetwork {
		return cap, true
	}
	m.addGovernanceCapabilityFacts(taskID, cap)
	if cap.State == GovernanceStateNoNetwork {
		m.addFact(taskID, poc.Fact{Message: "local network is not initialized"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "run: miopunch init-network"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return cap, false
	}
	m.addFact(taskID, poc.Fact{Message: "current identity is not an admin"})
	m.addSuggestion(taskID, poc.Suggestion{Message: "run this action on an owner/admin node"})
	m.addSuggestion(taskID, poc.Suggestion{Message: "or create a distinct new network with: miopunch init-network --new --confirm create-new-network"})
	m.done(taskID, poc.ReasonCodeForbidden, poc.ExitCodeForbidden)
	return cap, false
}

func (m *Manager) addGovernanceCapabilityFacts(taskID string, cap localGovernanceCapability) {
	m.addFact(taskID, poc.Fact{TermID: "governance_state", Message: "governance_state=" + cap.State})
	if cap.SelfPeerID != "" {
		m.addFact(taskID, poc.Fact{TermID: "self_peer_id", Message: "self_peer_id=" + cap.SelfPeerID})
	}
	if cap.NetID != "" {
		m.addFact(taskID, poc.Fact{TermID: "net_id", Message: "net_id=" + cap.NetID})
	}
	if cap.Reason != "" {
		m.addFact(taskID, poc.Fact{Message: "governance_reason=" + cap.Reason})
	}
}
