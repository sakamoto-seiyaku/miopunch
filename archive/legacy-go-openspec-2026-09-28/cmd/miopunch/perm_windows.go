//go:build windows

package main

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
	return errors.Is(err, syscall.ERROR_ACCESS_DENIED) || errors.Is(err, syscall.ERROR_PRIVILEGE_NOT_HELD)
}
