package task

import (
	"encoding/json"
	"errors"
	"os"
	"strings"

	"github.com/miopunch/miopunch/internal/pocstate"
)

const (
	GovernanceStateNoNetwork             = "no_network"
	GovernanceStateAdminNetwork          = "admin_network"
	GovernanceStateMemberNetwork         = "member_network"
	GovernanceStateForeignOrStaleNetwork = "foreign_or_stale_network"
)

type localGovernanceCapability struct {
	State               string
	SelfPeerID          string
	SelfRole            string
	NetID               string
	GovernanceHeadB64   string
	DeclsHeadB64        string
	Reason              string
	CanInitOwner        bool
	CanCreateNewNetwork bool
	CanInvite           bool
	CanApprove          bool
}

func (m *Manager) localGovernanceCapability(stateDir string, selfID pocstate.Identity) localGovernanceCapability {
	cap := localGovernanceCapability{
		State:      GovernanceStateForeignOrStaleNetwork,
		SelfPeerID: strings.TrimSpace(selfID.PeerID),
		SelfRole:   "unknown",
		Reason:     "local governance state is incomplete or stale",
	}

	netState, netErr := pocstate.LoadNet(stateDir)
	if netErr == nil {
		cap.NetID = strings.TrimSpace(netState.NetID)
	} else if !errors.Is(netErr, os.ErrNotExist) {
		cap.Reason = "load net: " + netErr.Error()
		cap.CanCreateNewNetwork = true
		return cap
	}

	head, headErr := pocstate.LoadGovernanceHeadSnapshot(stateDir)
	if headErr == nil {
		cap.GovernanceHeadB64 = strings.TrimSpace(head.HashB64)
		if head.IsOwner(selfID.PeerID) {
			cap.SelfRole = "owner"
		} else if head.IsAdmin(selfID.PeerID) {
			cap.SelfRole = "admin"
		}
	} else if !errors.Is(headErr, os.ErrNotExist) {
		cap.Reason = "load governance head: " + headErr.Error()
		cap.CanCreateNewNetwork = true
		return cap
	}

	decls, declsErr := pocstate.LoadDecls(stateDir)
	if declsErr == nil {
		cap.DeclsHeadB64 = strings.TrimSpace(decls.DeclsHeadB64)
	} else if !errors.Is(declsErr, os.ErrNotExist) {
		cap.Reason = "load decls: " + declsErr.Error()
		cap.CanCreateNewNetwork = true
		return cap
	}

	netMissing := errors.Is(netErr, os.ErrNotExist)
	headMissing := errors.Is(headErr, os.ErrNotExist)
	declsMissing := errors.Is(declsErr, os.ErrNotExist)
	if netMissing && headMissing && (declsMissing || len(decls.Decls) == 0) {
		cap.State = GovernanceStateNoNetwork
		cap.Reason = "no local network has been initialized"
		cap.CanInitOwner = true
		return cap
	}

	if netErr != nil || headErr != nil {
		cap.CanCreateNewNetwork = true
		return cap
	}

	if head.IsOwner(selfID.PeerID) || head.IsAdmin(selfID.PeerID) {
		cap.State = GovernanceStateAdminNetwork
		cap.Reason = "current identity is an owner/admin"
		cap.CanInvite = true
		cap.CanApprove = true
		return cap
	}

	if localDeclsApprovePeer(decls, selfID.PeerID) {
		cap.State = GovernanceStateMemberNetwork
		cap.SelfRole = "member"
		cap.Reason = "current identity is a member, not an admin"
		cap.CanCreateNewNetwork = true
		return cap
	}

	cap.State = GovernanceStateForeignOrStaleNetwork
	cap.Reason = "current identity is not an admin of the local network"
	cap.CanCreateNewNetwork = true
	return cap
}

func localDeclsApprovePeer(decls pocstate.DeclsFileV0, peerID string) bool {
	peerID = strings.TrimSpace(peerID)
	if peerID == "" {
		return false
	}
	for _, decl := range decls.Decls {
		if strings.TrimSpace(decl.Kind) != pocstate.DeclKindApproveMember {
			continue
		}
		var body pocstate.ApproveMemberBodyV0
		if err := json.Unmarshal(decl.Body, &body); err != nil {
			continue
		}
		if strings.TrimSpace(body.MemberPeerID) == peerID {
			return true
		}
	}
	return false
}
