//go:build !windows

package desktopbridge

import (
	"errors"
	"os"
	"syscall"
)

func isPermissionError(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, os.ErrPermission) {
		return true
	}
	return errors.Is(err, syscall.EACCES) || errors.Is(err, syscall.EPERM)
}
