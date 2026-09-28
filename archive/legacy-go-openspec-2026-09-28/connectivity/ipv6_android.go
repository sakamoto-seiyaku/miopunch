// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//go:build android

package connectivity

import (
	"fmt"
	"net/netip"

	"github.com/miopunch/miopunch/nat"
)

func gatherLocalIPv6IfaceAddrs() ([]IPv6IfaceAddr, error) {
	ips, err := nat.ListAllLocalIPs()
	if err != nil {
		return nil, fmt.Errorf("android local ip provider: %w", err)
	}

	out := make([]IPv6IfaceAddr, 0, len(ips))
	for _, ip := range ips {
		addr, ok := netip.AddrFromSlice(ip)
		if !ok || !addr.Is6() || addr.Is4In6() {
			continue
		}
		out = append(out, IPv6IfaceAddr{
			IfName:  "android-netlink",
			IfIndex: 0,
			Prefix:  netip.PrefixFrom(addr, 128),
			Addr:    addr,
		})
	}
	return out, nil
}
