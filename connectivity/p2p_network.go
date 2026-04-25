package connectivity

import (
	"fmt"
	"strings"
)

// P2PNetwork controls whether a session attempts TCP, UDP, or both.
//
// It MUST NOT be applied to signaling connectivity (coord/mqtt).
type P2PNetwork string

const (
	P2PNetworkAuto    P2PNetwork = "auto"
	P2PNetworkUDPOnly P2PNetwork = "udp_only"
	P2PNetworkTCPOnly P2PNetwork = "tcp_only"
)

func ParseP2PNetwork(value string) (P2PNetwork, error) {
	value = strings.TrimSpace(strings.ToLower(value))
	switch value {
	case "", "auto":
		return P2PNetworkAuto, nil
	case "u", "udp", "udp_only":
		return P2PNetworkUDPOnly, nil
	case "t", "tcp", "tcp_only":
		return P2PNetworkTCPOnly, nil
	default:
		return "", fmt.Errorf("invalid p2p network: %q", value)
	}
}

func MergeP2PNetwork(a, b P2PNetwork) (P2PNetwork, error) {
	pa, err := ParseP2PNetwork(string(a))
	if err != nil {
		return "", err
	}
	pb, err := ParseP2PNetwork(string(b))
	if err != nil {
		return "", err
	}

	if pa == pb {
		return pa, nil
	}
	if pa == P2PNetworkAuto {
		return pb, nil
	}
	if pb == P2PNetworkAuto {
		return pa, nil
	}
	return "", fmt.Errorf("p2p_network mismatch: %s vs %s", pa, pb)
}
