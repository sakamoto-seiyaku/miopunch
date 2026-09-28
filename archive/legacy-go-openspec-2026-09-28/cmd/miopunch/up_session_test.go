//go:build !windows

package main

import (
	"bytes"
	"context"
	"errors"
	"os"
	"strings"
	"syscall"
	"testing"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
)

func TestDefaultLocalAPIConflict_DetectsReachableUserDaemon(t *testing.T) {
	restore := replaceProbeLocalAPI(t, func(_ context.Context, addr localapi.Addr) error {
		if strings.Contains(addr.String(), "user.sock") {
			return nil
		}
		return errors.New("not reachable")
	})
	defer restore()

	var stderr bytes.Buffer
	failure, ok := defaultLocalAPIConflict(
		context.Background(),
		localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/system.sock"},
		localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/user.sock"},
		nil,
		&stderr,
	)

	if !ok {
		t.Fatalf("defaultLocalAPIConflict() ok = false, want true")
	}
	if failure.ReasonCode != poc.ReasonCodeConflict {
		t.Fatalf("failure.ReasonCode = %q, want %q", failure.ReasonCode, poc.ReasonCodeConflict)
	}
	if !strings.Contains(failure.Facts[0].Message, "user localapi is reachable") {
		t.Fatalf("failure.Facts = %#v, want user localapi conflict", failure.Facts)
	}
}

func TestDefaultLocalAPIConflict_SystemPermissionDoesNotBlockUserSession(t *testing.T) {
	restore := replaceProbeLocalAPI(t, func(_ context.Context, addr localapi.Addr) error {
		if strings.Contains(addr.String(), "system.sock") {
			return os.ErrPermission
		}
		return errors.New("not reachable")
	})
	defer restore()

	var stderr bytes.Buffer
	_, ok := defaultLocalAPIConflict(
		context.Background(),
		localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/system.sock"},
		localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/user.sock"},
		nil,
		&stderr,
	)

	if ok {
		t.Fatalf("defaultLocalAPIConflict() ok = true, want false")
	}
	if !strings.Contains(stderr.String(), "continuing with user session LocalAPI") {
		t.Fatalf("stderr = %q, want user session fallback diagnostic", stderr.String())
	}
}

func TestDefaultLocalAPIConflict_UserPermissionBlocksStartup(t *testing.T) {
	restore := replaceProbeLocalAPI(t, func(_ context.Context, addr localapi.Addr) error {
		if strings.Contains(addr.String(), "user.sock") {
			return syscall.EACCES
		}
		return errors.New("not reachable")
	})
	defer restore()

	var stderr bytes.Buffer
	failure, ok := defaultLocalAPIConflict(
		context.Background(),
		localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/system.sock"},
		localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/user.sock"},
		nil,
		&stderr,
	)

	if !ok {
		t.Fatalf("defaultLocalAPIConflict() ok = false, want true")
	}
	if failure.ReasonCode != poc.ReasonCodeForbidden {
		t.Fatalf("failure.ReasonCode = %q, want %q", failure.ReasonCode, poc.ReasonCodeForbidden)
	}
}

func replaceProbeLocalAPI(t *testing.T, fn func(context.Context, localapi.Addr) error) func() {
	t.Helper()

	old := probeLocalAPI
	probeLocalAPI = fn
	return func() {
		probeLocalAPI = old
	}
}
