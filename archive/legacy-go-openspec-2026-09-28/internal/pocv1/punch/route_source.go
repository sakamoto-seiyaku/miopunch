// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

package punch

import (
	"net/netip"
	"strings"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/logutil"
)

func augmentLocalPathMaterialFromPeerDirect(
	cfg LoadedConfig,
	snapshot UDPSnapshot,
	peerDirectAddrs []string,
	dialID string,
	label string,
) (LoadedConfig, UDPSnapshot) {
	derived := connectivity.DeriveUDPLocalSourceCandidates(
		peerDirectAddrs,
		cfg.UDPConn,
		cfg.UDP6Conn,
		cfg.P2PIPFamily,
	)
	if len(derived) == 0 {
		logutil.Debugf("%s route-source: dial_id=%s derived=0 peer_direct_addrs=%v", label, dialID, peerDirectAddrs)
		return cfg, snapshot
	}

	cfg = mergeDerivedLocalCandidates(cfg, derived, dialID, label)
	for _, ap := range derived {
		snapshot.DirectAddrs = append(snapshot.DirectAddrs, ap.String())
	}
	normalized, err := normalizeUDPSnapshot(snapshot)
	if err != nil {
		logutil.Debugf("%s route-source snapshot normalize failed: dial_id=%s derived=%v err=%v", label, dialID, derived, err)
		return cfg, snapshot
	}
	logutil.Debugf(
		"%s route-source snapshot augmented: dial_id=%s derived=%v direct_addrs=%v assisted_addrs=%v",
		label,
		dialID,
		derived,
		normalized.DirectAddrs,
		normalized.AssistedAddrs,
	)
	return cfg, normalized
}

func augmentLocalCandidatesFromPeerDirect(
	cfg LoadedConfig,
	peerDirectAddrs []string,
	dialID string,
	label string,
) LoadedConfig {
	derived := connectivity.DeriveUDPLocalSourceCandidates(
		peerDirectAddrs,
		cfg.UDPConn,
		cfg.UDP6Conn,
		cfg.P2PIPFamily,
	)
	if len(derived) == 0 {
		logutil.Debugf("%s route-source candidates: dial_id=%s derived=0 peer_direct_addrs=%v", label, dialID, peerDirectAddrs)
		return cfg
	}
	return mergeDerivedLocalCandidates(cfg, derived, dialID, label)
}

func mergeDerivedLocalCandidates(
	cfg LoadedConfig,
	derived []netip.AddrPort,
	dialID string,
	label string,
) LoadedConfig {
	candidates := append([]Candidate(nil), cfg.LocalCandidates...)
	for _, ap := range derived {
		candidates = append(candidates, Candidate{
			Kind: CandidateKindHost,
			Addr: ap.String(),
		})
	}

	normalized, err := normalizeOptionalCandidatesForIPFamily(candidates, cfg.P2PIPFamily)
	if err != nil {
		logutil.Debugf("%s route-source candidate normalize failed: dial_id=%s derived=%v err=%v", label, dialID, derived, err)
		return cfg
	}
	cfg.LocalCandidates = normalized
	logutil.Debugf(
		"%s route-source candidates augmented: dial_id=%s derived=%v local_candidates=%s",
		label,
		dialID,
		derived,
		formatCandidates(cfg.LocalCandidates),
	)
	return cfg
}

func peerDirectAddrStrings(snapshot UDPSnapshot, candidates []Candidate) []string {
	out := make([]string, 0, len(snapshot.DirectAddrs)+len(snapshot.AssistedAddrs)+len(candidates))
	out = append(out, snapshot.DirectAddrs...)
	out = append(out, snapshot.AssistedAddrs...)
	for _, candidate := range candidates {
		if candidate.Kind != CandidateKindHost {
			continue
		}
		out = append(out, candidate.Addr)
	}

	seen := map[string]struct{}{}
	deduped := make([]string, 0, len(out))
	for _, raw := range out {
		addr := strings.TrimSpace(raw)
		if addr == "" {
			continue
		}
		if _, ok := seen[addr]; ok {
			continue
		}
		seen[addr] = struct{}{}
		deduped = append(deduped, addr)
	}
	return deduped
}
