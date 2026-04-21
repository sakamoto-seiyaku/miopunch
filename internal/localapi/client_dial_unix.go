//go:build !windows

package localapi

import (
	"context"
	"fmt"
	"net"
)

func dialContextForAddr(addr Addr) (func(ctx context.Context, network string, address string) (net.Conn, error), error) {
	if addr.Transport != TransportUnix {
		return nil, fmt.Errorf("unsupported transport: %q", addr.Transport)
	}
	socketPath := addr.Path
	return func(ctx context.Context, _ string, _ string) (net.Conn, error) {
		var d net.Dialer
		return d.DialContext(ctx, "unix", socketPath)
	}, nil
}
