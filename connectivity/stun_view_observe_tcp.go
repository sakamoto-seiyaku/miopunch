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

func observeInternalSTUNViewTCP(ctx context.Context, dialer *net.Dialer, resolver *netutil.DNSResolver, servers []string, localIPs []string) *wire.STUNViewObservation {
	out := &wire.STUNViewObservation{}

	resolved, resolveErrors := resolveInternalSTUNServersTCP(ctx, resolver, servers)
	stunRes := discoverInternalSTUNTCP(ctx, dialer, resolved)
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

func resolveInternalSTUNServersTCP(ctx context.Context, resolver *netutil.DNSResolver, servers []string) (resolved []string, errors []string) {
	usable, _, filterErrors := stunclient.FilterHostPorts(servers, stunclient.EndpointSchemeTCP)
	resolved, resolveErrors := stunclient.ResolveHostPortsIP4(ctx, resolver, usable, internalSTUNResolvedEndpointLimit)

	errors = make([]string, 0, len(filterErrors)+len(resolveErrors))
	errors = append(errors, filterErrors...)
	errors = append(errors, resolveErrors...)
	return resolved, errors
}

func resolveSTUNServersTCP(ctx context.Context, resolver *netutil.DNSResolver, servers []string) (resolved []string, ignored []string, errors []string) {
	usable, ignored, filterErrors := stunclient.FilterHostPorts(servers, stunclient.EndpointSchemeTCP)
	resolved, resolveErrors := stunclient.ResolveHostPortsIP4(ctx, resolver, usable, 0)

	errors = make([]string, 0, len(filterErrors)+len(resolveErrors))
	errors = append(errors, filterErrors...)
	errors = append(errors, resolveErrors...)
	return resolved, ignored, errors
}

