// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

package punch

import (
	"context"
	"fmt"
	"net"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/logutil"
	legacywire "github.com/miopunch/miopunch/internal/wire"
)

func gatherUDPSnapshotDefault(ctx context.Context, cfg LoadedConfig, sid string) (UDPSnapshot, error) {
	gather, err := connectivity.Gather(ctx, sid, connectivity.GatherConfig{
		UDP4Conn:                   cfg.UDPConn,
		UDP6Conn:                   cfg.UDP6Conn,
		P2PNetwork:                 cfg.P2PNetwork,
		P2PIPFamily:                cfg.P2PIPFamily,
		DisableAssistedAddrs:       false,
		DisableSTUNViewArbitration: true,
		StunServers:                cfg.StunServers,
		StunExplicit:               cfg.StunExplicit,
		StunTimeout:                cfg.StunTimeout,
	})
	if err != nil {
		return UDPSnapshot{}, fmt.Errorf("gather udp snapshot: %w", err)
	}
	logutil.Debugf(
		"punch udp snapshot gathered: sid=%s p2p_network=%s p2p_ip_family=%s udp4_local_addr=%s udp6_local_addr=%s direct_addrs=%v mapped_addrs=%v assisted_addrs=%v",
		sid,
		cfg.P2PNetwork,
		cfg.P2PIPFamily,
		addrString(cfg.UDPConn),
		addrString(cfg.UDP6Conn),
		gather.DirectAddrs,
		gather.MappedAddrs,
		gather.AssistedAddrs,
	)
	return normalizeUDPSnapshot(UDPSnapshot{
		DirectAddrs:   gather.DirectAddrs,
		MappedAddrs:   gather.MappedAddrs,
		AssistedAddrs: gather.AssistedAddrs,
	})
}

func addrString(conn *net.UDPConn) string {
	if conn == nil || conn.LocalAddr() == nil {
		return ""
	}
	return conn.LocalAddr().String()
}

func visitorSnapshot(dialID string, snapshot UDPSnapshot, p2pNetwork connectivity.P2PNetwork) *legacywire.NatHoleVisitor {
	return &legacywire.NatHoleVisitor{
		TransactionID: dialID,
		Protocol:      "kcp",
		P2PNetwork:    string(p2pNetwork),
		DirectAddrs:   append([]string(nil), snapshot.DirectAddrs...),
		MappedAddrs:   append([]string(nil), snapshot.MappedAddrs...),
		AssistedAddrs: append([]string(nil), snapshot.AssistedAddrs...),
	}
}

func clientSnapshot(dialID string, snapshot UDPSnapshot, p2pNetwork connectivity.P2PNetwork) *legacywire.NatHoleClient {
	return &legacywire.NatHoleClient{
		TransactionID: dialID,
		Sid:           dialID,
		Protocol:      "kcp",
		P2PNetwork:    string(p2pNetwork),
		DirectAddrs:   append([]string(nil), snapshot.DirectAddrs...),
		MappedAddrs:   append([]string(nil), snapshot.MappedAddrs...),
		AssistedAddrs: append([]string(nil), snapshot.AssistedAddrs...),
	}
}
