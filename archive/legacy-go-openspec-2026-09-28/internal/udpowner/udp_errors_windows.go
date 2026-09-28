//go:build windows

package udpowner

import (
	"errors"
	"syscall"
)

const (
	wsaECONNRESET   syscall.Errno = 10054
	wsaECONNREFUSED syscall.Errno = 10061
)

func recoverableSocketReadError(err error) bool {
	return errors.Is(err, wsaECONNREFUSED) || errors.Is(err, wsaECONNRESET)
}
