//go:build aix || android || darwin || dragonfly || freebsd || linux || netbsd || openbsd || solaris

package udpowner

import (
	"errors"
	"syscall"
)

func recoverableSocketReadError(err error) bool {
	return errors.Is(err, syscall.ECONNREFUSED) || errors.Is(err, syscall.ECONNRESET)
}
