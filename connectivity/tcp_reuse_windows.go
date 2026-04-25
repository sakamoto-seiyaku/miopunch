//go:build windows

package connectivity

import "syscall"

func tcpReuseControl(network, address string, c syscall.RawConn) error {
	var controlErr error
	if err := c.Control(func(fd uintptr) {
		controlErr = syscall.SetsockoptInt(int(fd), syscall.SOL_SOCKET, syscall.SO_REUSEADDR, 1)
	}); err != nil {
		return err
	}
	return controlErr
}

