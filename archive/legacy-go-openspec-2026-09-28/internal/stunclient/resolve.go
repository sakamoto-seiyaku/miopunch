package stunclient

import (
	"context"
	"fmt"
	"net"
	"net/netip"
	"strings"

	"github.com/miopunch/miopunch/internal/netutil"
)

// ResolveHostPortsIP4 resolves hostnames in a list of host:port entries into
// IP literals (best-effort) using ip4 resolution.
//
// Behavior is intentionally aligned with existing connectivity STUN resolution:
//   - If net.SplitHostPort fails, the entry is kept as-is (so the caller can
//     surface a later dial/resolve error).
//   - For hostnames, at most two A records are expanded.
//   - If resolver is nil, hostname resolution is skipped with an error entry.
//   - If limit > 0, resolution stops after producing that many resolved entries.
func ResolveHostPortsIP4(ctx context.Context, resolver *netutil.DNSResolver, hostPorts []string, limit int) (resolved []string, errors []string) {
	resolved = make([]string, 0, len(hostPorts))
	errors = make([]string, 0)

	for _, raw := range hostPorts {
		if limit > 0 && len(resolved) >= limit {
			break
		}

		server := strings.TrimSpace(raw)
		if server == "" {
			errors = append(errors, "<empty>: empty stun server")
			continue
		}

		host, port, err := net.SplitHostPort(server)
		if err != nil {
			resolved = append(resolved, server)
			continue
		}

		host = strings.Trim(host, "[]")
		if _, err := netip.ParseAddr(host); err == nil {
			resolved = append(resolved, net.JoinHostPort(host, port))
			continue
		}

		if resolver == nil {
			errors = append(errors, fmt.Sprintf("%s: resolve: no dns resolver configured", server))
			continue
		}

		addrs, err := resolver.LookupNetIP(ctx, "ip4", host)
		if err != nil {
			errors = append(errors, fmt.Sprintf("%s: resolve: %v", server, err))
			continue
		}

		max := 2
		if remaining := limit - len(resolved); limit > 0 && remaining < max {
			max = remaining
		}
		if len(addrs) < max {
			max = len(addrs)
		}
		for i := 0; i < max; i++ {
			resolved = append(resolved, net.JoinHostPort(addrs[i].String(), port))
		}
	}

	return resolved, errors
}
