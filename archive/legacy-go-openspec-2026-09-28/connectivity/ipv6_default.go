// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//go:build !android

package connectivity

import (
	"net"
	"net/netip"
	"strings"
)

func gatherLocalIPv6IfaceAddrs() ([]IPv6IfaceAddr, error) {
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
			if !ok || !na.Is6() || na.Is4In6() {
				continue
			}
			out = append(out, IPv6IfaceAddr{
				IfName:  iface.Name,
				IfIndex: iface.Index,
				Prefix:  netip.PrefixFrom(na, ones),
				Addr:    na,
			})
		}
	}
	return out, nil
}
