//go:build desktop && linux

package main

import (
	"errors"
	"os"
)

func validatePlatformStartupEnvironment() error {
	if os.Getenv("DISPLAY") != "" || os.Getenv("WAYLAND_DISPLAY") != "" {
		return nil
	}
	return errors.New("no Linux desktop display found: DISPLAY and WAYLAND_DISPLAY are empty")
}
