package desktopbridge

import (
	"errors"
	"strings"
	"testing"

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
