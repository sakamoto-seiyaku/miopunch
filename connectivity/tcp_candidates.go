package connectivity

import (
	"net"
	"net/netip"
	"strconv"
)

var ipv4CGNATPrefix = netip.MustParsePrefix("100.64.0.0/10")

func isIPv4CGNAT(addr netip.Addr) bool {
	return addr.Is4() && ipv4CGNATPrefix.Contains(addr)
}

func isInvalidLocalIPv4Addr(addr netip.Addr) bool {
	return addr.IsLoopback() || addr.IsUnspecified() || addr.IsLinkLocalUnicast() || addr.IsMulticast()
}

func isTCPAssistedIPv4ListenAddr(addr netip.Addr) bool {
	return addr.Is4() && !isInvalidLocalIPv4Addr(addr) && (addr.IsPrivate() || isIPv4CGNAT(addr))
}

func isTCPDirectIPv4ListenAddr(addr netip.Addr) bool {
	return addr.Is4() && !isInvalidLocalIPv4Addr(addr) && !addr.IsPrivate() && !isIPv4CGNAT(addr)
}

func isTCPDirectIPv4PortmapAddr(addr netip.Addr) bool {
	return addr.Is4() && !isInvalidLocalIPv4Addr(addr) && !addr.IsPrivate()
}

type tcp4ListenBuckets struct {
	DirectAddrs     []netip.AddrPort
	AssistedAddrs   []string
	RejectedSources []string
}

func classifyTCP4ListenCandidates(localIPs []string, listenPort int) tcp4ListenBuckets {
	out := tcp4ListenBuckets{
		DirectAddrs:     make([]netip.AddrPort, 0, len(localIPs)),
		AssistedAddrs:   make([]string, 0, len(localIPs)),
		RejectedSources: make([]string, 0),
	}
	if listenPort <= 0 {
		return out
	}

	for _, rawIP := range localIPs {
		addr, err := netip.ParseAddr(rawIP)
		if err != nil || !addr.Is4() {
			out.RejectedSources = append(out.RejectedSources, rawIP)
			continue
		}

		switch {
		case isTCPDirectIPv4ListenAddr(addr):
			out.DirectAddrs = append(out.DirectAddrs, netip.AddrPortFrom(addr, uint16(listenPort)))
		case isTCPAssistedIPv4ListenAddr(addr):
			out.AssistedAddrs = append(out.AssistedAddrs, net.JoinHostPort(rawIP, strconv.Itoa(listenPort)))
		default:
			out.RejectedSources = append(out.RejectedSources, rawIP)
		}
	}
	return out
}

func filterTCPPortmapDirectAddrs(in []netip.AddrPort) (valid []netip.AddrPort, dropped []netip.AddrPort) {
	valid = make([]netip.AddrPort, 0, len(in))
	dropped = make([]netip.AddrPort, 0)
	for _, ap := range in {
		addr := ap.Addr()
		if !addr.Is4() {
			valid = append(valid, ap)
			continue
		}
		if !isTCPDirectIPv4PortmapAddr(addr) {
			dropped = append(dropped, ap)
			continue
		}
		valid = append(valid, ap)
	}
	return valid, dropped
}
