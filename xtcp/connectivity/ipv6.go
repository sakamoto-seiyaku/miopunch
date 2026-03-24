package connectivity

import (
	"net"
	"net/netip"
	"slices"
	"strings"
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

	// Prefer global unicast; only keep ULA when no global address exists.
	hasGlobal := false
	for _, addr := range trimmed {
		if !isIPv6ULA(addr) {
			hasGlobal = true
			break
		}
	}
	if hasGlobal {
		tmp := trimmed[:0]
		for _, addr := range trimmed {
			if isIPv6ULA(addr) {
				continue
			}
			tmp = append(tmp, addr)
		}
		trimmed = tmp
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
	ifaces, err := net.Interfaces()
	if err != nil {
		return nil, err
	}

	out := make([]IPv6IfaceAddr, 0, 16)
	for _, iface := range ifaces {
		if (iface.Flags & net.FlagUp) == 0 {
			continue
		}
		if (iface.Flags & net.FlagLoopback) != 0 {
			continue
		}
		if strings.HasPrefix(iface.Name, "zt") || iface.Name == "wt0" {
			continue
		}

		addrs, err := iface.Addrs()
		if err != nil {
			continue
		}

		for _, addr := range addrs {
			var (
				ip   net.IP
				ones int
			)
			switch v := addr.(type) {
			case *net.IPNet:
				ip = v.IP
				ones, _ = v.Mask.Size()
			case *net.IPAddr:
				ip = v.IP
				ones = 128
			default:
				continue
			}

			na, ok := netip.AddrFromSlice(ip)
			if !ok || !na.Is6() {
				continue
			}
			if na.Is4In6() {
				continue
			}
			pfx := netip.PrefixFrom(na, ones)
			out = append(out, IPv6IfaceAddr{
				IfName:  iface.Name,
				IfIndex: iface.Index,
				Prefix:  pfx,
				Addr:    na,
			})
		}
	}

	return FilterIPv6Candidates(out), nil
}
