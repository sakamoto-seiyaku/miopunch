package connectivity

import (
	"context"
	"fmt"
	"net"

	"github.com/miopunch/miopunch/internal/netutil"
	"github.com/miopunch/miopunch/internal/stunclient"
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

func observeInternalSTUNView(ctx context.Context, client *stunclient.UDPClient, resolver *netutil.DNSResolver, servers []string, localIPs []string) *wire.STUNViewObservation {
	out := &wire.STUNViewObservation{}

	resolved, resolveErrors := resolveInternalSTUNServers(ctx, resolver, servers)
	stunRes := discoverInternalSTUN(ctx, client, resolved)
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

func resolveInternalSTUNServers(ctx context.Context, resolver *netutil.DNSResolver, servers []string) (resolved []string, errors []string) {
	usable, _, filterErrors := stunclient.FilterHostPorts(servers, stunclient.EndpointSchemeUDP)
	resolved, resolveErrors := stunclient.ResolveHostPortsIP4(ctx, resolver, usable, internalSTUNResolvedEndpointLimit)

	errors = make([]string, 0, len(filterErrors)+len(resolveErrors))
	errors = append(errors, filterErrors...)
	errors = append(errors, resolveErrors...)
	return resolved, errors
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
	usable, _, filterErrors := stunclient.FilterHostPorts(servers, stunclient.EndpointSchemeUDP)
	resolved, resolveErrors := stunclient.ResolveHostPortsIP4(ctx, resolver, usable, 0)

	errors = make([]string, 0, len(filterErrors)+len(resolveErrors))
	errors = append(errors, filterErrors...)
	errors = append(errors, resolveErrors...)
	return resolved, errors
}
