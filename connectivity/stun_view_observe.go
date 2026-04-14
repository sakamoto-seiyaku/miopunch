package connectivity

import (
	"context"
	"fmt"
	"net"
	"net/netip"
	"strings"

	"github.com/miopunch/miopunch/internal/netutil"
	"github.com/miopunch/miopunch/internal/wire"
	"github.com/miopunch/miopunch/nat"
)

func observeSTUNView(ctx context.Context, conn *net.UDPConn, resolver *netutil.DNSResolver, servers []string, localIPs []string) *wire.STUNViewObservation {
	out := &wire.STUNViewObservation{}

	resolved, resolveErrors := resolveSTUNServers(ctx, resolver, servers)
	stunRes := DiscoverSTUN(ctx, conn, resolved)
	stunRes.Errors = append(stunRes.Errors, resolveErrors...)

	out.OkCount = stunRes.OkCount
	out.RTTMs = stunRes.RTTMs
	out.MappedAddrs = stunRes.MappedAddrs
	out.Errors = stunRes.Errors

	if len(out.MappedAddrs) < 2 {
		out.Available = false
		out.NATDifficulty = 999
		return out
	}

	feature, err := nat.ClassifyNATFeature(out.MappedAddrs, localIPs)
	if err != nil {
		out.Available = false
		out.NATDifficulty = 999
		out.Errors = append(out.Errors, fmt.Sprintf("classify_nat_feature: %v", err))
		return out
	}
	out.Available = true
	out.NATDifficulty = natDifficulty(feature)
	return out
}

func natDifficulty(f *nat.NatFeature) int {
	if f == nil {
		return 999
	}
	if f.PublicNetwork {
		return 0
	}
	switch f.NatType {
	case nat.EasyNAT:
		return 1
	case nat.HardNAT:
		if f.RegularPortsChange {
			return 2
		}
		switch f.Behavior {
		case nat.BehaviorPortChanged:
			return 3
		case nat.BehaviorIPChanged:
			return 4
		case nat.BehaviorBothChanged:
			return 5
		default:
			return 6
		}
	default:
		return 999
	}
}

func resolveSTUNServers(ctx context.Context, resolver *netutil.DNSResolver, servers []string) (resolved []string, errors []string) {
	resolved = make([]string, 0, len(servers))
	errors = make([]string, 0)
	for _, raw := range servers {
		server := normalizeSTUNServer(raw)
		host, port, err := net.SplitHostPort(server)
		if err != nil {
			resolved = append(resolved, server)
			continue
		}
		if _, err := netip.ParseAddr(host); err == nil {
			resolved = append(resolved, net.JoinHostPort(host, port))
			continue
		}

		addrs, err := resolver.LookupNetIP(ctx, "ip4", host)
		if err != nil {
			errors = append(errors, fmt.Sprintf("%s: resolve: %v", server, err))
			continue
		}
		max := 2
		if len(addrs) < max {
			max = len(addrs)
		}
		for i := 0; i < max; i++ {
			resolved = append(resolved, net.JoinHostPort(addrs[i].String(), port))
		}
	}
	return resolved, errors
}

func normalizeSTUNServer(server string) string {
	server = strings.TrimSpace(server)
	server = strings.TrimPrefix(server, "udp://")
	server = strings.TrimPrefix(server, "tcp://")
	if i := strings.Index(server, "?"); i >= 0 {
		server = server[:i]
	}
	return server
}

