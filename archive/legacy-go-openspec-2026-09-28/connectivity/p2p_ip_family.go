package connectivity

import (
	"fmt"
	"strings"
)

// P2PIPFamily constrains only the peer-to-peer punching/attempt path selection.
// It MUST NOT be applied to signaling connectivity (coord/mqtt).
type P2PIPFamily string

const (
	P2PIPFamilyAuto P2PIPFamily = "auto"
	P2PIPFamilyV4   P2PIPFamily = "v4"
	P2PIPFamilyV6   P2PIPFamily = "v6"
)

func ParseP2PIPFamily(value string) (P2PIPFamily, error) {
	value = strings.TrimSpace(strings.ToLower(value))
	switch value {
	case "", "auto":
		return P2PIPFamilyAuto, nil
	case "4", "v4", "ipv4":
		return P2PIPFamilyV4, nil
	case "6", "v6", "ipv6":
		return P2PIPFamilyV6, nil
	default:
		return "", fmt.Errorf("invalid p2p ip family: %q", value)
	}
}
