// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

package connectivity

import (
	"fmt"
	"net"
	"net/netip"

	"github.com/miopunch/miopunch/internal/logutil"
)

func DeriveUDPLocalSourceCandidates(
	peerDirectAddrs []string,
	udp4Conn *net.UDPConn,
	udp6Conn *net.UDPConn,
	p2pIPFamily P2PIPFamily,
) []netip.AddrPort {
	family, err := ParseP2PIPFamily(string(p2pIPFamily))
	if err != nil {
		logutil.Debugf("punch route-source derive skipped invalid ip family: p2p_ip_family=%q err=%v", p2pIPFamily, err)
		family = P2PIPFamilyAuto
	}
	allowV4 := family != P2PIPFamilyV6
	allowV6 := family != P2PIPFamilyV4

	udp4Port := 0
	if udp4Conn != nil {
		if port, err := udpPort(udp4Conn.LocalAddr()); err == nil {
			udp4Port = port
		}
	}
	udp6Port := 0
	if udp6Conn != nil {
		if port, err := udpPort(udp6Conn.LocalAddr()); err == nil {
			udp6Port = port
		}
	}

	parsed := ParseDirectAddrPorts(peerDirectAddrs)
	if len(parsed.Invalid) > 0 {
		logutil.Debugf("punch route-source derive dropped invalid peer direct addrs: invalid=%v", parsed.Invalid)
	}

	out := make([]netip.AddrPort, 0, len(parsed.Addrs))
	for _, peer := range parsed.Addrs {
		switch {
		case peer.Addr().Is4() && allowV4 && udp4Port > 0:
			addr, err := deriveUDPLocalSourceAddr("udp4", peer)
			if err != nil {
				logutil.Debugf("punch route-source derive failed: peer=%s network=udp4 err=%v", peer, err)
				continue
			}
			if !isUsableRouteSourceAddr(addr) || !addr.Is4() {
				logutil.Debugf("punch route-source derive skipped unusable source: peer=%s source=%s", peer, addr)
				continue
			}
			out = append(out, netip.AddrPortFrom(addr, uint16(udp4Port)))
		case peer.Addr().Is6() && allowV6 && udp6Port > 0:
			addr, err := deriveUDPLocalSourceAddr("udp6", peer)
			if err != nil {
				logutil.Debugf("punch route-source derive failed: peer=%s network=udp6 err=%v", peer, err)
				continue
			}
			if !isUsableRouteSourceAddr(addr) || !addr.Is6() || addr.Is4In6() {
				logutil.Debugf("punch route-source derive skipped unusable source: peer=%s source=%s", peer, addr)
				continue
			}
			out = append(out, netip.AddrPortFrom(addr, uint16(udp6Port)))
		default:
			logutil.Debugf(
				"punch route-source derive skipped peer: peer=%s p2p_ip_family=%s udp4_port=%d udp6_port=%d",
				peer,
				family,
				udp4Port,
				udp6Port,
			)
		}
	}

	out = TrimDirectAddrPorts(out)
	logutil.Debugf("punch route-source derive complete: peer_count=%d candidate_count=%d candidates=%v", len(parsed.Addrs), len(out), out)
	return out
}

func deriveUDPLocalSourceAddr(network string, peer netip.AddrPort) (netip.Addr, error) {
	remote := &net.UDPAddr{
		IP:   net.IP(peer.Addr().AsSlice()),
		Port: int(peer.Port()),
	}
	conn, err := net.DialUDP(network, nil, remote)
	if err != nil {
		return netip.Addr{}, fmt.Errorf("dial udp route: %w", err)
	}
	defer conn.Close() // best-effort close

	local, ok := conn.LocalAddr().(*net.UDPAddr)
	if !ok || local == nil || local.IP == nil {
		return netip.Addr{}, fmt.Errorf("local addr is not UDPAddr")
	}
	return netIPToAddr(local.IP)
}

func netIPToAddr(ip net.IP) (netip.Addr, error) {
	if ip4 := ip.To4(); ip4 != nil {
		addr, ok := netip.AddrFromSlice(ip4)
		if !ok {
			return netip.Addr{}, fmt.Errorf("invalid ipv4 source")
		}
		return addr, nil
	}
	addr, ok := netip.AddrFromSlice(ip)
	if !ok {
		return netip.Addr{}, fmt.Errorf("invalid ip source")
	}
	return addr.Unmap(), nil
}

func isUsableRouteSourceAddr(addr netip.Addr) bool {
	if !addr.IsValid() {
		return false
	}
	if addr.IsLoopback() || addr.IsUnspecified() || addr.IsMulticast() {
		return false
	}
	if addr.IsLinkLocalUnicast() || addr.IsLinkLocalMulticast() {
		return false
	}
	return true
}
