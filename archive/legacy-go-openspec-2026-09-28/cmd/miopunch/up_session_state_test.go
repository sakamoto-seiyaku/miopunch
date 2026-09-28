package main

import (
	"errors"
	"path/filepath"
	"testing"
)

func TestApplySessionStatePathUsesPortableDefault(t *testing.T) {
	want := filepath.Join(t.TempDir(), "data", "state.json")
	got, err := applySessionStatePathWithResolver(upOptions{Session: true}, func() (string, error) {
		return want, nil
	})
	if err != nil {
		t.Fatalf("applySessionStatePathWithResolver(session) error = %v", err)
	}
	if got.StatePath != want {
		t.Fatalf("applySessionStatePathWithResolver(session).StatePath = %q, want %q", got.StatePath, want)
	}
}

func TestApplySessionStatePathPreservesExplicitOverride(t *testing.T) {
	const want = "/tmp/custom/state.json"
	called := false
	got, err := applySessionStatePathWithResolver(upOptions{Session: true, StatePath: want}, func() (string, error) {
		called = true
		return "/tmp/portable/state.json", nil
	})
	if err != nil {
		t.Fatalf("applySessionStatePathWithResolver(explicit state path) error = %v", err)
	}
	if called {
		t.Fatalf("applySessionStatePathWithResolver(explicit state path) called resolver, want not called")
	}
	if got.StatePath != want {
		t.Fatalf("applySessionStatePathWithResolver(explicit state path).StatePath = %q, want %q", got.StatePath, want)
	}
}

func TestApplySessionStatePathIgnoresNonSession(t *testing.T) {
	called := false
	got, err := applySessionStatePathWithResolver(upOptions{}, func() (string, error) {
		called = true
		return "/tmp/portable/state.json", nil
	})
	if err != nil {
		t.Fatalf("applySessionStatePathWithResolver(non-session) error = %v", err)
	}
	if called {
		t.Fatalf("applySessionStatePathWithResolver(non-session) called resolver, want not called")
	}
	if got.StatePath != "" {
		t.Fatalf("applySessionStatePathWithResolver(non-session).StatePath = %q, want empty", got.StatePath)
	}
}

func TestApplySessionStatePathReturnsResolverError(t *testing.T) {
	_, err := applySessionStatePathWithResolver(upOptions{Session: true}, func() (string, error) {
		return "", errors.New("boom")
	})
	if err == nil {
		t.Fatalf("applySessionStatePathWithResolver(resolver error) error = nil, want error")
	}
}
