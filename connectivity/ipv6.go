package connectivity

import (
	"net/netip"
	"slices"
)

var ulaPrefix = netip.MustParsePrefix("fc00::/7")

type IPv6IfaceAddr struct {
	IfName  string
	IfIndex int
	Prefix  netip.Prefix
	Addr    netip.Addr
}

func isIPv6ULA(addr netip.Addr) bool {
	return addr.Is6() && ulaPrefix.Contains(addr)
}

func FilterIPv6Candidates(in []IPv6IfaceAddr) []netip.Addr {
	type subnetKey struct {
		ifIndex int
		prefix  netip.Prefix
	}

	bySubnet := make(map[subnetKey][]netip.Addr)
	for _, item := range in {
		addr := item.Addr
		if !addr.Is6() {
			continue
		}

		if addr.IsLoopback() || addr.IsUnspecified() || addr.IsMulticast() || addr.IsLinkLocalUnicast() || addr.IsLinkLocalMulticast() {
			continue
		}

		pfx := item.Prefix
		if !pfx.IsValid() {
			pfx = netip.PrefixFrom(addr, 128)
		}
		pfx = pfx.Masked()

		key := subnetKey{ifIndex: item.IfIndex, prefix: pfx}
		bySubnet[key] = append(bySubnet[key], addr)
	}

	trimmed := make([]netip.Addr, 0, len(in))
	for _, addrs := range bySubnet {
		slices.SortFunc(addrs, func(a, b netip.Addr) int {
			if a.Less(b) {
				return -1
			}
			if b.Less(a) {
				return 1
			}
			return 0
		})
		if len(addrs) > 2 {
			addrs = addrs[:2]
		}
		trimmed = append(trimmed, addrs...)
	}

	slices.SortFunc(trimmed, func(a, b netip.Addr) int {
		if a.Less(b) {
			return -1
		}
		if b.Less(a) {
			return 1
		}
		return 0
	})
	if len(trimmed) > MaxDirectAddrsV6 {
		trimmed = trimmed[:MaxDirectAddrsV6]
	}
	return trimmed
}

func GatherLocalIPv6Candidates() ([]netip.Addr, error) {
	out, err := gatherLocalIPv6IfaceAddrs()
	if err != nil {
		return nil, err
	}
	return FilterIPv6Candidates(out), nil
}
