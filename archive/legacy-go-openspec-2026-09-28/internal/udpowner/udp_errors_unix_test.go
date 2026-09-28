//go:build aix || android || darwin || dragonfly || freebsd || linux || netbsd || openbsd || solaris

package udpowner

import (
	"os"
	"syscall"
	"testing"
)

func TestRecoverableSocketReadErrorUnix(t *testing.T) {
	tests := []struct {
		name string
		err  error
	}{
		{
			name: "connection refused",
			err:  &os.SyscallError{Syscall: "recvfrom", Err: syscall.ECONNREFUSED},
		},
		{
			name: "connection reset",
			err:  &os.SyscallError{Syscall: "recvfrom", Err: syscall.ECONNRESET},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if !recoverableUDPReadError(tt.err) {
				t.Fatalf("recoverableUDPReadError(%v) = false, want true", tt.err)
			}
		})
	}
}
