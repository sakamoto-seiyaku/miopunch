//go:build windows

package localapi

import (
	"context"
	"fmt"
	"net"

	"github.com/Microsoft/go-winio"
)

func dialContextForAddr(addr Addr) (func(ctx context.Context, network string, address string) (net.Conn, error), error) {
	if addr.Transport != TransportNpipe {
		return nil, fmt.Errorf("unsupported transport: %q", addr.Transport)
	}
	pipePath := addr.Path
	return func(ctx context.Context, _ string, _ string) (net.Conn, error) {
		return winio.DialPipeContext(ctx, pipePath)
	}, nil
}
