package main

import (
	"errors"
	"path/filepath"
	"testing"
)

func TestResolveRuntimeRootWithUserHome_PrefersExplicitStatePath(t *testing.T) {
	t.Parallel()

	got, err := resolveRuntimeRootWithUserHome("/tmp/state/state.json", func() (string, error) {
		return "", errors.New("unexpected")
	}, func() (string, error) {
		return "", errors.New("unexpected")
	})
	if err != nil {
		t.Fatalf("resolveRuntimeRootWithUserHome(explicit state path) error = %v, want nil", err)
	}
	want := filepath.Clean("/tmp/state")
	if got != want {
		t.Fatalf("resolveRuntimeRootWithUserHome(explicit state path) = %q, want %q", got, want)
	}
}

func TestResolveRuntimeRootWithUserHome_FallsBackToCurrentUserLookup(t *testing.T) {
	t.Parallel()

	got, err := resolveRuntimeRootWithUserHome("", func() (string, error) {
		return "", errors.New("$HOME is not defined")
	}, func() (string, error) {
		return "/root", nil
	})
	if err != nil {
		t.Fatalf("resolveRuntimeRootWithUserHome(fallback) error = %v, want nil", err)
	}
	want := filepath.Join("/root", ".local", "state", "miopunch", "pocv1")
	if got != want {
		t.Fatalf("resolveRuntimeRootWithUserHome(fallback) = %q, want %q", got, want)
	}
}
