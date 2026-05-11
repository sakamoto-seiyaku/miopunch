//go:build desktop && linux

package main

import "testing"

func TestValidatePlatformStartupEnvironmentRequiresDisplay(t *testing.T) {
	t.Setenv("DISPLAY", "")
	t.Setenv("WAYLAND_DISPLAY", "")

	if err := validatePlatformStartupEnvironment(); err == nil {
		t.Fatalf("validatePlatformStartupEnvironment() error = nil, want error")
	}
}

func TestValidatePlatformStartupEnvironmentAllowsX11(t *testing.T) {
	t.Setenv("DISPLAY", ":0")
	t.Setenv("WAYLAND_DISPLAY", "")

	if err := validatePlatformStartupEnvironment(); err != nil {
		t.Fatalf("validatePlatformStartupEnvironment() error = %v, want nil", err)
	}
}

func TestValidatePlatformStartupEnvironmentAllowsWayland(t *testing.T) {
	t.Setenv("DISPLAY", "")
	t.Setenv("WAYLAND_DISPLAY", "wayland-0")

	if err := validatePlatformStartupEnvironment(); err != nil {
		t.Fatalf("validatePlatformStartupEnvironment() error = %v, want nil", err)
	}
}
