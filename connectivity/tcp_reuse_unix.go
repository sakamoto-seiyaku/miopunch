//go:build darwin || linux || freebsd || netbsd || openbsd || dragonfly

package connectivity

import (
	"syscall"

	"golang.org/x/sys/unix"
)

func tcpReuseControl(network, address string, c syscall.RawConn) error {
	var controlErr error
	if err := c.Control(func(fd uintptr) {
		if err := unix.SetsockoptInt(int(fd), unix.SOL_SOCKET, unix.SO_REUSEADDR, 1); err != nil {
			controlErr = err
			return
		}
		// Best-effort: some platforms may not support reuseport in all contexts.
		_ = unix.SetsockoptInt(int(fd), unix.SOL_SOCKET, unix.SO_REUSEPORT, 1)
	}); err != nil {
		return err
	}
	return controlErr
}
