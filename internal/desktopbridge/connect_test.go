package desktopbridge

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
)

func TestClassifyProbeFailure_Permission(t *testing.T) {
	t.Parallel()

	addr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/miopunch.sock"}
	got := classifyProbeFailure(addr, "system", probeFailure{kind: probeFailurePermission, err: errors.New("permission denied")})
	if got == nil {
		t.Fatalf("classifyProbeFailure() = nil, want non-nil")
	}
	if got.ReasonCode != poc.ReasonCodeForbidden {
		t.Fatalf("ReasonCode = %q, want %q", got.ReasonCode, poc.ReasonCodeForbidden)
	}
	if got.ExitCode != poc.ExitCodeForbidden {
		t.Fatalf("ExitCode = %d, want %d", got.ExitCode, poc.ExitCodeForbidden)
	}
	if !strings.Contains(strings.ToLower(got.Message), "permission") {
		t.Fatalf("Message = %q, want permission hint", got.Message)
	}
	if len(got.Suggestions) == 0 {
		t.Fatalf("Suggestions = empty, want non-empty")
	}
}

func TestClassifyProbeFailure_Incompatible(t *testing.T) {
	t.Parallel()

	addr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/miopunch.sock"}
	got := classifyProbeFailure(addr, "user", probeFailure{kind: probeFailureIncompatible, err: errors.New("unexpected status 404")})
	if got == nil {
		t.Fatalf("classifyProbeFailure() = nil, want non-nil")
	}
	if got.ReasonCode != poc.ReasonCodeUnavailable {
		t.Fatalf("ReasonCode = %q, want %q", got.ReasonCode, poc.ReasonCodeUnavailable)
	}
	if got.ExitCode != poc.ExitCodeUnavailable {
		t.Fatalf("ExitCode = %d, want %d", got.ExitCode, poc.ExitCodeUnavailable)
	}
	if len(got.Suggestions) == 0 {
		t.Fatalf("Suggestions = empty, want non-empty")
	}
}

func TestClassifyNoEndpointFailure_PrefersIncompatible(t *testing.T) {
	t.Parallel()

	systemAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/system.sock"}
	userAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/user.sock"}

	got := classifyNoEndpointFailure(
		systemAddr, nil, probeFailure{kind: probeFailureIncompatible, err: errors.New("unexpected status")},
		userAddr, nil, probeFailure{kind: probeFailureUnreachable, err: errors.New("dial refused")},
	)
	if got == nil {
		t.Fatalf("classifyNoEndpointFailure() = nil, want non-nil")
	}
	if got.ReasonCode != poc.ReasonCodeUnavailable {
		t.Fatalf("ReasonCode = %q, want %q", got.ReasonCode, poc.ReasonCodeUnavailable)
	}
}

func TestConnectWithOptions_ReusesUserLocalAPIBeforeBootstrap(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())

	started := false
	_, _, state, managed := connectWithOptions(context.Background(), connectOptions{
		probeLocalAPI: func(_ context.Context, addr localapi.Addr) (*localapi.Client, probeFailure) {
			if strings.Contains(addr.String(), "localapi.sock") {
				return nil, probeFailure{kind: probeFailureOK}
			}
			return nil, probeFailure{kind: probeFailureUnreachable, err: errors.New("unreachable")}
		},
		startDaemon: func(string) (*ManagedDaemon, error) {
			started = true
			return &ManagedDaemon{done: make(chan error)}, nil
		},
	})

	if !state.Connected {
		t.Fatalf("connectWithOptions() Connected = false, want true")
	}
	if state.Selected != EndpointUser {
		t.Fatalf("connectWithOptions() Selected = %q, want %q", state.Selected, EndpointUser)
	}
	if state.DesktopManaged {
		t.Fatalf("connectWithOptions() DesktopManaged = true, want false")
	}
	if started {
		t.Fatalf("connectWithOptions() started daemon, want reused daemon")
	}
	if managed != nil {
		t.Fatalf("connectWithOptions() managed daemon = %v, want nil", managed)
	}
}

func TestConnectWithOptions_BootstrapsSameUserDaemon(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())

	started := false
	_, _, state, managed := connectWithOptions(context.Background(), connectOptions{
		probeLocalAPI: func(_ context.Context, addr localapi.Addr) (*localapi.Client, probeFailure) {
			if strings.Contains(addr.String(), "/run/miopunch") {
				return nil, probeFailure{kind: probeFailurePermission, err: errors.New("permission denied")}
			}
			if started {
				return nil, probeFailure{kind: probeFailureOK}
			}
			return nil, probeFailure{kind: probeFailureUnreachable, err: errors.New("not ready")}
		},
		resolveDaemonPath: func() (string, error) {
			return "/tmp/miopunch", nil
		},
		startDaemon: func(string) (*ManagedDaemon, error) {
			started = true
			return &ManagedDaemon{done: make(chan error)}, nil
		},
		bootstrapTimeout:  50 * time.Millisecond,
		bootstrapInterval: time.Millisecond,
	})

	if !state.Connected {
		t.Fatalf("connectWithOptions() Connected = false, want true")
	}
	if state.Selected != EndpointUser {
		t.Fatalf("connectWithOptions() Selected = %q, want %q", state.Selected, EndpointUser)
	}
	if state.Bootstrap != BootstrapReady {
		t.Fatalf("connectWithOptions() Bootstrap = %q, want %q", state.Bootstrap, BootstrapReady)
	}
	if !state.DesktopManaged {
		t.Fatalf("connectWithOptions() DesktopManaged = false, want true")
	}
	if managed == nil {
		t.Fatalf("connectWithOptions() managed daemon = nil, want non-nil")
	}
	if !hasFact(state.Diagnostics, "system_probe_error=permission denied") {
		t.Fatalf("connectWithOptions() diagnostics = %#v, want system permission fact", state.Diagnostics)
	}
}

func TestConnectWithOptions_OverrideBypassesBootstrap(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())

	started := false
	_, _, state, managed := connectWithOptions(context.Background(), connectOptions{
		overrideAddr: "unix:/tmp/override.sock",
		probeLocalAPI: func(_ context.Context, addr localapi.Addr) (*localapi.Client, probeFailure) {
			if addr.String() != "unix:/tmp/override.sock" {
				t.Fatalf("probe addr = %q, want override only", addr.String())
			}
			return nil, probeFailure{kind: probeFailureOK}
		},
		startDaemon: func(string) (*ManagedDaemon, error) {
			started = true
			return &ManagedDaemon{done: make(chan error)}, nil
		},
	})

	if !state.Connected {
		t.Fatalf("connectWithOptions(override) Connected = false, want true")
	}
	if state.Selected != EndpointOverride {
		t.Fatalf("connectWithOptions(override) Selected = %q, want %q", state.Selected, EndpointOverride)
	}
	if started {
		t.Fatalf("connectWithOptions(override) started daemon, want no bootstrap")
	}
	if managed != nil {
		t.Fatalf("connectWithOptions(override) managed daemon = %v, want nil", managed)
	}
}

func TestConnectWithOptions_BootstrapFailureUsesSessionGuidance(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())

	_, _, state, _ := connectWithOptions(context.Background(), connectOptions{
		probeLocalAPI: func(_ context.Context, addr localapi.Addr) (*localapi.Client, probeFailure) {
			if strings.Contains(addr.String(), "/run/miopunch") {
				return nil, probeFailure{kind: probeFailurePermission, err: errors.New("permission denied")}
			}
			return nil, probeFailure{kind: probeFailureUnreachable, err: errors.New("not ready")}
		},
		resolveDaemonPath: func() (string, error) {
			return "/tmp/miopunch", nil
		},
		startDaemon: func(string) (*ManagedDaemon, error) {
			return &ManagedDaemon{done: make(chan error)}, nil
		},
		bootstrapTimeout:  time.Millisecond,
		bootstrapInterval: time.Millisecond,
	})

	if state.Connected {
		t.Fatalf("connectWithOptions() Connected = true, want false")
	}
	if state.Failure == nil {
		t.Fatalf("connectWithOptions() Failure = nil, want bootstrap failure")
	}
	if state.Failure.ReasonCode != poc.ReasonCodeUnavailable {
		t.Fatalf("Failure.ReasonCode = %q, want %q", state.Failure.ReasonCode, poc.ReasonCodeUnavailable)
	}
	for _, suggestion := range state.Failure.Suggestions {
		if strings.Contains(suggestion.Message, "install-system-daemon") {
			t.Fatalf("Failure.Suggestions = %#v, want no install-system-daemon guidance", state.Failure.Suggestions)
		}
	}
}

func hasFact(facts []poc.Fact, value string) bool {
	for _, fact := range facts {
		if fact.Message == value {
			return true
		}
	}
	return false
}
