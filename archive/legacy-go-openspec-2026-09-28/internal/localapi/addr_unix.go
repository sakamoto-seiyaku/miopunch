//go:build !windows

package localapi

import (
	"net"
	"os"

	"github.com/miopunch/miopunch/internal/poc"
)

func DefaultSystemAddr(_ string) (Addr, error) {
	return Addr{
		Transport: TransportUnix,
		Path:      systemUnixSocketPath(),
	}, nil
}

func DefaultUserAddr(_ string) (Addr, error) {
	path, err := userUnixSocketPath()
	if err != nil {
		return Addr{}, err
	}
	return Addr{
		Transport: TransportUnix,
		Path:      path,
	}, nil
}

func Listen(addr Addr, mode ListenMode) (net.Listener, error) {
	dirMode := os.FileMode(0o700)
	socketMode := os.FileMode(0o600)
	groupName := ""
	if mode == ListenModeSystem {
		dirMode = 0o750
		socketMode = 0o660
		groupName = poc.LinuxOperatorGroup
	}

	return listenUnixSocket(unixListenOptions{
		socketPath: addr.Path,
		dirMode:    dirMode,
		socketMode: socketMode,
		groupName:  groupName,
	})
}
