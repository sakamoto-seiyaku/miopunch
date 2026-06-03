package runtime

import (
	"strings"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocv1/session"
)

type peerPathPolicy struct {
	P2PNetwork  connectivity.P2PNetwork
	P2PIPFamily connectivity.P2PIPFamily
}

func defaultPeerPathPolicy() peerPathPolicy {
	return peerPathPolicy{
		P2PNetwork:  connectivity.P2PNetworkAuto,
		P2PIPFamily: connectivity.P2PIPFamilyAuto,
	}
}

func normalizePeerPathPolicy(networkValue string, familyValue string) (peerPathPolicy, *problem) {
	network, err := connectivity.ParseP2PNetwork(networkValue)
	if err != nil {
		return peerPathPolicy{}, wrapProblem(
			StageSecureSession,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"invalid p2p network",
			err,
			"retry with --p2p-network auto|udp_only|tcp_only",
		)
	}
	family, err := connectivity.ParseP2PIPFamily(familyValue)
	if err != nil {
		return peerPathPolicy{}, wrapProblem(
			StageSecureSession,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"invalid p2p ip family",
			err,
			"retry with --p2p-ip-family auto|v4|v6",
		)
	}
	return peerPathPolicy{P2PNetwork: network, P2PIPFamily: family}, nil
}

func (p peerPathPolicy) explicit() bool {
	return p.P2PNetwork != "" && p.P2PNetwork != connectivity.P2PNetworkAuto ||
		p.P2PIPFamily != "" && p.P2PIPFamily != connectivity.P2PIPFamilyAuto
}

func (p peerPathPolicy) unsupportedTCPOnlyProblem() *problem {
	if p.P2PNetwork != connectivity.P2PNetworkTCPOnly {
		return nil
	}
	return newProblem(
		StagePunch,
		poc.ReasonCodeNotImplemented,
		poc.ExitCodeBadRequest,
		"current POC v1 does not support tcp_only P2P path establishment",
		[]poc.Fact{{Message: "p2p_network=tcp_only"}},
		[]poc.Suggestion{{Message: "retry with -u or omit -t"}},
	)
}

func (p peerPathPolicy) matchesSession(sess session.PeerSession) bool {
	if sess == nil || !sess.Healthy() {
		return false
	}
	if !p.explicit() {
		return true
	}

	key := sess.Key().Normalize()
	selectedPath := strings.TrimSpace(dataplane.PathFactsFromSession(sess).SelectedPath)

	switch p.P2PNetwork {
	case "", connectivity.P2PNetworkAuto:
	case connectivity.P2PNetworkUDPOnly:
		if key.Protocol != "" && key.Protocol != dataplane.ProtocolKCP {
			return false
		}
		if key.PathFamily != dataplane.PathFamilyUDP4 && key.PathFamily != dataplane.PathFamilyUDP6 {
			return false
		}
	case connectivity.P2PNetworkTCPOnly:
		return false
	}

	switch p.P2PIPFamily {
	case "", connectivity.P2PIPFamilyAuto:
		return true
	case connectivity.P2PIPFamilyV4:
		return key.PathFamily == dataplane.PathFamilyUDP4 ||
			strings.Contains(selectedPath, "ipv4") ||
			strings.Contains(selectedPath, "tcp4")
	case connectivity.P2PIPFamilyV6:
		return key.PathFamily == dataplane.PathFamilyUDP6 ||
			strings.Contains(selectedPath, "ipv6") ||
			strings.Contains(selectedPath, "tcp6")
	default:
		return false
	}
}
