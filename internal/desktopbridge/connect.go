package desktopbridge

import (
	"context"
	"errors"
	"runtime"
	"strings"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
)

func Connect(ctx context.Context, overrideAddr string) (*localapi.Client, localapi.Addr, ConnectionState) {
	state := ConnectionState{
		Connected:    false,
		Selected:     EndpointNone,
		OverrideAddr: strings.TrimSpace(overrideAddr),
	}

	operatorSID, err := poc.CurrentOperatorSID()
	if err != nil {
		state.Failure = &BridgeError{
			Stage:      "desktop",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Message:    "failed to determine operator identity",
			Facts: []poc.Fact{
				{Message: "error=" + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		}
		return nil, localapi.Addr{}, state
	}

	systemAddr, systemAddrErr := localapi.DefaultSystemAddr(operatorSID)
	if systemAddrErr == nil {
		state.SystemAddr = systemAddr.String()
	}

	userAddr, userAddrErr := localapi.DefaultUserAddr(operatorSID)
	if userAddrErr == nil {
		state.UserAddr = userAddr.String()
	}

	if state.OverrideAddr != "" {
		addr, err := localapi.ParseAddr(state.OverrideAddr)
		if err != nil {
			state.Failure = &BridgeError{
				Stage:      "desktop",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Message:    "invalid LocalAPI override address",
				Facts: []poc.Fact{
					{Message: "override_addr=" + state.OverrideAddr},
					{Message: "error=" + err.Error()},
				},
				Suggestions: []poc.Suggestion{
					{Message: "use: unix:/path/to/localapi.sock"},
					{Message: `or: npipe:\\.\pipe\miopunch\localapi-<operator_sid>`},
				},
			}
			return nil, localapi.Addr{}, state
		}

		c, probe := probeLocalAPI(ctx, addr)
		if probe.kind == probeFailureOK {
			state.Connected = true
			state.Selected = EndpointOverride
			state.Addr = addr.String()
			return c, addr, state
		}

		state.Failure = classifyProbeFailure(addr, "override", probe)
		return nil, localapi.Addr{}, state
	}

	var systemProbe probeFailure
	if systemAddrErr == nil {
		c, probe := probeLocalAPI(ctx, systemAddr)
		if probe.kind == probeFailureOK {
			state.Connected = true
			state.Selected = EndpointSystem
			state.Addr = systemAddr.String()
			return c, systemAddr, state
		}

		systemProbe = probe
		if systemProbe.kind == probeFailurePermission {
			state.Failure = classifyProbeFailure(systemAddr, "system", systemProbe)
			return nil, localapi.Addr{}, state
		}
	}

	var userProbe probeFailure
	if userAddrErr == nil {
		c, probe := probeLocalAPI(ctx, userAddr)
		if probe.kind == probeFailureOK {
			state.Connected = true
			state.Selected = EndpointUser
			state.Addr = userAddr.String()
			return c, userAddr, state
		}

		userProbe = probe
		if userProbe.kind == probeFailurePermission {
			state.Failure = classifyProbeFailure(userAddr, "user", userProbe)
			return nil, localapi.Addr{}, state
		}
	}

	state.Failure = classifyNoEndpointFailure(systemAddr, systemAddrErr, systemProbe, userAddr, userAddrErr, userProbe)
	return nil, localapi.Addr{}, state
}

type probeFailureKind int

const (
	probeFailureOK probeFailureKind = iota
	probeFailurePermission
	probeFailureIncompatible
	probeFailureUnreachable
	probeFailureUnknown
)

type probeFailure struct {
	kind probeFailureKind
	err  error
}

func probeLocalAPI(ctx context.Context, addr localapi.Addr) (*localapi.Client, probeFailure) {
	c, err := localapi.NewClient(addr)
	if err != nil {
		return nil, probeFailure{kind: probeFailureUnknown, err: err}
	}

	if err := c.ProbeStatus(ctx); err == nil {
		return c, probeFailure{kind: probeFailureOK}
	} else if isPermissionError(err) {
		return nil, probeFailure{kind: probeFailurePermission, err: err}
	}

	var statusErr *localapi.UnexpectedStatusError
	if errors.As(err, &statusErr) {
		return nil, probeFailure{kind: probeFailureIncompatible, err: err}
	}

	return nil, probeFailure{kind: probeFailureUnreachable, err: err}
}

func classifyProbeFailure(addr localapi.Addr, endpoint string, probe probeFailure) *BridgeError {
	if probe.kind == probeFailurePermission {
		return &BridgeError{
			Stage:      "desktop",
			ReasonCode: poc.ReasonCodeForbidden,
			ExitCode:   poc.ExitCodeForbidden,
			Message:    "permission denied connecting to LocalAPI",
			Facts: []poc.Fact{
				{Message: "endpoint=" + endpoint},
				{Message: "addr=" + addr.String()},
			},
			Suggestions: permissionSuggestions(endpoint),
		}
	}

	if probe.kind == probeFailureIncompatible {
		return &BridgeError{
			Stage:      "desktop",
			ReasonCode: poc.ReasonCodeUnavailable,
			ExitCode:   poc.ExitCodeUnavailable,
			Message:    "unexpected response from LocalAPI (possible version mismatch)",
			Facts: []poc.Fact{
				{Message: "endpoint=" + endpoint},
				{Message: "addr=" + addr.String()},
				{Message: "error=" + probe.err.Error()},
			},
			Suggestions: incompatibleSuggestions(),
		}
	}

	return &BridgeError{
		Stage:      "desktop",
		ReasonCode: poc.ReasonCodeDaemonNotRunning,
		ExitCode:   poc.ExitCodeUnavailable,
		Message:    "LocalAPI is not reachable",
		Facts: []poc.Fact{
			{Message: "endpoint=" + endpoint},
			{Message: "addr=" + addr.String()},
			{Message: "error=" + errorString(probe.err)},
		},
		Suggestions: daemonNotRunningSuggestions(),
	}
}

func classifyNoEndpointFailure(
	systemAddr localapi.Addr,
	systemAddrErr error,
	systemProbe probeFailure,
	userAddr localapi.Addr,
	userAddrErr error,
	userProbe probeFailure,
) *BridgeError {
	facts := []poc.Fact{}

	if systemAddrErr == nil {
		facts = append(facts, poc.Fact{Message: "system_addr=" + systemAddr.String()})
	} else if systemAddrErr != nil {
		facts = append(facts, poc.Fact{Message: "system_addr_error=" + systemAddrErr.Error()})
	}

	if userAddrErr == nil {
		facts = append(facts, poc.Fact{Message: "user_addr=" + userAddr.String()})
	} else if userAddrErr != nil {
		facts = append(facts, poc.Fact{Message: "user_addr_error=" + userAddrErr.Error()})
	}

	if systemProbe.err != nil {
		facts = append(facts, poc.Fact{Message: "system_probe_error=" + systemProbe.err.Error()})
	}
	if userProbe.err != nil {
		facts = append(facts, poc.Fact{Message: "user_probe_error=" + userProbe.err.Error()})
	}

	if systemProbe.kind == probeFailureIncompatible || userProbe.kind == probeFailureIncompatible {
		return &BridgeError{
			Stage:       "desktop",
			ReasonCode:  poc.ReasonCodeUnavailable,
			ExitCode:    poc.ExitCodeUnavailable,
			Message:     "unexpected response from LocalAPI (possible version mismatch)",
			Facts:       facts,
			Suggestions: incompatibleSuggestions(),
		}
	}

	return &BridgeError{
		Stage:       "desktop",
		ReasonCode:  poc.ReasonCodeDaemonNotRunning,
		ExitCode:    poc.ExitCodeUnavailable,
		Message:     "LocalAPI is not reachable",
		Facts:       facts,
		Suggestions: daemonNotRunningSuggestions(),
	}
}

func errorString(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}

func permissionSuggestions(endpoint string) []poc.Suggestion {
	if runtime.GOOS == "windows" {
		switch endpoint {
		case "system":
			return []poc.Suggestion{
				{Message: "run miopunch-desktop as the same Windows user that installed the service"},
				{Message: "or reinstall the system service as the intended operator"},
			}
		default:
			return []poc.Suggestion{
				{Message: "run miopunch-desktop as the same Windows user that owns the LocalAPI pipe"},
				{Message: "or reinstall the system service as the intended operator"},
			}
		}
	}

	if endpoint == "system" {
		return []poc.Suggestion{
			{Message: "join the operator group: " + poc.LinuxOperatorGroup},
			{Message: "log out and log back in"},
		}
	}

	return []poc.Suggestion{
		{Message: "check socket permissions"},
	}
}

func daemonNotRunningSuggestions() []poc.Suggestion {
	if runtime.GOOS == "windows" {
		return []poc.Suggestion{
			{Message: "start the daemon: miopunch up"},
			{Message: "or install system service: miopunch install-system-daemon"},
		}
	}

	return []poc.Suggestion{
		{Message: "start the daemon: miopunch up"},
		{Message: "or install system service: sudo miopunch install-system-daemon"},
	}
}

func incompatibleSuggestions() []poc.Suggestion {
	if runtime.GOOS == "windows" {
		return []poc.Suggestion{
			{Message: "repair/reinstall via the installer"},
			{Message: `export installer log: %ProgramData%\\miopunch\\install.log`},
		}
	}

	return []poc.Suggestion{
		{Message: "repair/reinstall via the package manager"},
		{Message: "export installer log: /var/log/miopunch/install.log"},
	}
}
