//go:build windows

package localapi

import (
	"net"
	"strings"
)

func DefaultSystemAddr(operatorSID string) (Addr, error) {
	path, err := windowsPipePath(operatorSID)
	if err != nil {
		return Addr{}, err
	}
	return Addr{
		Transport:   TransportNpipe,
		Path:        path,
		OperatorSID: strings.TrimSpace(operatorSID),
	}, nil
}

func DefaultUserAddr(operatorSID string) (Addr, error) {
	return DefaultSystemAddr(operatorSID)
}

func Listen(addr Addr, _ ListenMode) (net.Listener, error) {
	return listenWindowsPipe(addr.Path, addr.OperatorSID)
}
