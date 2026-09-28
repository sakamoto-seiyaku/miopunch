package desktopbridge

import (
	"context"
	"errors"
	"fmt"
	"runtime"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/bundlepath"
	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
)

const (
	defaultBootstrapTimeout  = 15 * time.Second
	defaultBootstrapInterval = 100 * time.Millisecond
)

type connectOptions struct {
	overrideAddr string

	probeLocalAPI             func(context.Context, localapi.Addr) (*localapi.Client, probeFailure)
	resolveDaemonPath         func() (string, error)
	resolveBootstrapProbeAddr func(string, localapi.Addr) (localapi.Addr, error)
	startDaemon               func(string) (*ManagedDaemon, error)

	bootstrapTimeout  time.Duration
	bootstrapInterval time.Duration
}

func Connect(ctx context.Context, overrideAddr string) (*localapi.Client, localapi.Addr, ConnectionState, *ManagedDaemon) {
	return connectWithOptions(ctx, connectOptions{overrideAddr: overrideAddr})
}

func connectWithOptions(ctx context.Context, opt connectOptions) (*localapi.Client, localapi.Addr, ConnectionState, *ManagedDaemon) {
	if ctx == nil {
		ctx = context.Background()
	}
	if opt.probeLocalAPI == nil {
		opt.probeLocalAPI = probeLocalAPI
	}
	if opt.resolveDaemonPath == nil {
		opt.resolveDaemonPath = resolveSiblingDaemonPath
	}
	if opt.resolveBootstrapProbeAddr == nil {
		opt.resolveBootstrapProbeAddr = resolveBootstrapProbeAddr
	}
	if opt.startDaemon == nil {
		opt.startDaemon = StartManagedDaemon
	}
	if opt.bootstrapTimeout <= 0 {
		opt.bootstrapTimeout = defaultBootstrapTimeout
	}
	if opt.bootstrapInterval <= 0 {
		opt.bootstrapInterval = defaultBootstrapInterval
	}

	state := ConnectionState{
		Connected:    false,
		Selected:     EndpointNone,
		Bootstrap:    BootstrapNone,
		OverrideAddr: strings.TrimSpace(opt.overrideAddr),
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
		return nil, localapi.Addr{}, state, nil
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
		addr, err := parseOverrideAddr(state.OverrideAddr)
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
			return nil, localapi.Addr{}, state, nil
		}

		c, probe := opt.probeLocalAPI(ctx, addr)
		if probe.kind == probeFailureOK {
			state.Connected = true
			state.Selected = EndpointOverride
			state.Addr = addr.String()
			return c, addr, state, nil
		}

		state.Failure = classifyProbeFailure(addr, "override", probe)
		return nil, localapi.Addr{}, state, nil
	}

	var userProbe probeFailure
	if userAddrErr == nil {
		c, probe := opt.probeLocalAPI(ctx, userAddr)
		if probe.kind == probeFailureOK {
			state.Connected = true
			state.Selected = EndpointUser
			state.Addr = userAddr.String()
			state.Diagnostics = append(state.Diagnostics, poc.Fact{Message: "selected_endpoint=user"})
			return c, userAddr, state, nil
		}

		userProbe = probe
		if userProbe.kind == probeFailurePermission {
			state.Failure = classifyProbeFailure(userAddr, "user", userProbe)
			return nil, localapi.Addr{}, state, nil
		}
	}

	var systemClient *localapi.Client
	var systemProbe probeFailure
	if systemAddrErr == nil {
		c, probe := opt.probeLocalAPI(ctx, systemAddr)
		systemProbe = probe
		if probe.kind == probeFailureOK {
			systemClient = c
		} else if probe.kind == probeFailurePermission {
			state.Diagnostics = append(state.Diagnostics, poc.Fact{
				Message: "system_probe_error=" + errorString(probe.err),
			})
		}
	}

	c, addr, managed, bootInfo, bootProbe := bootstrapSessionDaemon(ctx, userAddr, userAddrErr, opt)
	state.BootstrapInfo = bootInfo
	if managed != nil && bootProbe.kind == probeFailureOK {
		state.Connected = true
		state.Selected = EndpointUser
		state.Addr = addr.String()
		state.Bootstrap = BootstrapReady
		state.DesktopManaged = true
		state.Diagnostics = append(state.Diagnostics, poc.Fact{Message: "selected_endpoint=user"})
		return c, addr, state, managed
	}
	if bootInfo != nil && bootInfo.Attempted {
		state.Bootstrap = BootstrapFailed
		state.Diagnostics = append(state.Diagnostics, poc.Fact{Message: "bootstrap_error=" + bootInfo.Error})
		if bootInfo.Stage == "resolve_daemon" {
			state.Failure = classifyBootstrapFailure(userAddr, userAddrErr, bootInfo, bootProbe, nil)
			return nil, localapi.Addr{}, state, nil
		}
	}

	if systemClient != nil {
		state.Connected = true
		state.Selected = EndpointSystem
		state.Addr = systemAddr.String()
		state.Diagnostics = append(state.Diagnostics, poc.Fact{Message: "selected_endpoint=system"})
		return systemClient, systemAddr, state, nil
	}

	state.Failure = classifyNoEndpointFailure(systemAddr, systemAddrErr, systemProbe, userAddr, userAddrErr, userProbe)
	if bootInfo != nil && bootInfo.Attempted {
		state.Failure = classifyBootstrapFailure(userAddr, userAddrErr, bootInfo, bootProbe, state.Failure)
	}
	return nil, localapi.Addr{}, state, nil
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

func parseOverrideAddr(value string) (localapi.Addr, error) {
	return localapi.ParseAddr(value)
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

func bootstrapSessionDaemon(ctx context.Context, userAddr localapi.Addr, userAddrErr error, opt connectOptions) (*localapi.Client, localapi.Addr, *ManagedDaemon, *BootstrapDiagnostics, probeFailure) {
	info := &BootstrapDiagnostics{
		Attempted: true,
		Stage:     "resolve_daemon",
	}

	if userAddrErr != nil {
		info.Error = "failed to determine user localapi address: " + userAddrErr.Error()
		return nil, localapi.Addr{}, nil, info, probeFailure{kind: probeFailureUnknown, err: userAddrErr}
	}

	daemonPath, err := opt.resolveDaemonPath()
	if err != nil {
		info.Error = err.Error()
		return nil, localapi.Addr{}, nil, info, probeFailure{kind: probeFailureUnreachable, err: err}
	}
	info.DaemonPath = daemonPath
	probeAddr, err := opt.resolveBootstrapProbeAddr(daemonPath, userAddr)
	if err != nil {
		info.Error = err.Error()
		return nil, localapi.Addr{}, nil, info, probeFailure{kind: probeFailureUnreachable, err: err}
	}
	info.ProbeAddr = probeAddr.String()
	info.Stage = "start_daemon"

	managed, err := opt.startDaemon(daemonPath)
	if err != nil {
		info.Error = err.Error()
		return nil, localapi.Addr{}, nil, info, probeFailure{kind: probeFailureUnreachable, err: err}
	}
	info.PID = managed.PID()
	info.Stage = "wait_ready"

	deadline := time.NewTimer(opt.bootstrapTimeout)
	defer deadline.Stop()

	ticker := time.NewTicker(opt.bootstrapInterval)
	defer ticker.Stop()

	for {
		c, probe := opt.probeLocalAPI(ctx, probeAddr)
		if probe.kind == probeFailureOK {
			info.Stage = "ready"
			info.Stdout = managed.Stdout()
			info.Stderr = managed.Stderr()
			return c, probeAddr, managed, info, probe
		}

		if err, exited := managed.Exited(); exited {
			info.Stage = "failed"
			info.Stdout = managed.Stdout()
			info.Stderr = managed.Stderr()
			info.Error = "daemon exited before LocalAPI readiness"
			if err != nil {
				info.Error += ": " + err.Error()
			}
			return nil, localapi.Addr{}, nil, info, probeFailure{kind: probeFailureUnreachable, err: err}
		}

		select {
		case <-ctx.Done():
			_ = managed.Stop(context.Background())
			info.Stage = "failed"
			info.Stdout = managed.Stdout()
			info.Stderr = managed.Stderr()
			info.Error = ctx.Err().Error()
			return nil, localapi.Addr{}, nil, info, probeFailure{kind: probeFailureUnreachable, err: ctx.Err()}
		case <-deadline.C:
			_ = managed.Stop(context.Background())
			err := fmt.Errorf("timed out waiting for LocalAPI at %s", probeAddr.String())
			info.Stage = "timeout"
			info.Stdout = managed.Stdout()
			info.Stderr = managed.Stderr()
			info.Error = err.Error()
			return nil, localapi.Addr{}, nil, info, probeFailure{kind: probeFailureUnreachable, err: err}
		case <-ticker.C:
		}
	}
}

func resolveBootstrapProbeAddr(daemonPath string, fallback localapi.Addr) (localapi.Addr, error) {
	if runtime.GOOS == "windows" {
		return fallback, nil
	}

	path, err := bundlepath.LocalAPIPathForExecutable(daemonPath)
	if err != nil {
		return localapi.Addr{}, fmt.Errorf("resolve session LocalAPI path: %w", err)
	}
	return localapi.Addr{
		Transport: localapi.TransportUnix,
		Path:      path,
	}, nil
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

func classifyBootstrapFailure(
	userAddr localapi.Addr,
	userAddrErr error,
	info *BootstrapDiagnostics,
	probe probeFailure,
	fallback *BridgeError,
) *BridgeError {
	facts := []poc.Fact{
		{Message: "bootstrap_stage=" + info.Stage},
	}
	if userAddrErr == nil {
		facts = append(facts, poc.Fact{Message: "user_addr=" + userAddr.String()})
	}
	if info.DaemonPath != "" {
		facts = append(facts, poc.Fact{Message: "daemon_path=" + info.DaemonPath})
	}
	if info.ProbeAddr != "" {
		facts = append(facts, poc.Fact{Message: "bootstrap_addr=" + info.ProbeAddr})
	}
	if info.PID != 0 {
		facts = append(facts, poc.Fact{Message: fmt.Sprintf("pid=%d", info.PID)})
	}
	if info.Error != "" {
		facts = append(facts, poc.Fact{Message: "error=" + info.Error})
	}
	if info.Stderr != "" {
		facts = append(facts, poc.Fact{Message: "stderr=" + info.Stderr})
	}
	if probe.err != nil {
		facts = append(facts, poc.Fact{Message: "readiness_error=" + probe.err.Error()})
	}
	if fallback != nil {
		facts = append(facts, fallback.Facts...)
	}

	return &BridgeError{
		Stage:       "desktop",
		ReasonCode:  poc.ReasonCodeUnavailable,
		ExitCode:    poc.ExitCodeUnavailable,
		Message:     "same-user session daemon bootstrap failed",
		Facts:       facts,
		Suggestions: bootstrapFailureSuggestions(),
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
				{Message: "retry with a same-user session bundle"},
				{Message: "check the selected LocalAPI pipe owner"},
			}
		default:
			return []poc.Suggestion{
				{Message: "run miopunch-desktop as the same Windows user that owns the LocalAPI pipe"},
				{Message: "clear the override and retry the session connection"},
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
			{Message: "retry desktop connection"},
			{Message: "check that miopunch.exe is next to miopunch-desktop.exe"},
			{Message: "export runtime diagnostics"},
		}
	}

	return []poc.Suggestion{
		{Message: "retry desktop connection"},
		{Message: "check that ./miopunch is next to ./miopunch-desktop and executable"},
		{Message: "export runtime diagnostics"},
	}
}

func incompatibleSuggestions() []poc.Suggestion {
	if runtime.GOOS == "windows" {
		return []poc.Suggestion{
			{Message: "check that miopunch.exe and miopunch-desktop.exe came from the same bundle"},
			{Message: "export runtime diagnostics"},
		}
	}

	return []poc.Suggestion{
		{Message: "check that miopunch and miopunch-desktop came from the same bundle"},
		{Message: "export runtime diagnostics"},
	}
}

func bootstrapFailureSuggestions() []poc.Suggestion {
	if runtime.GOOS == "windows" {
		return []poc.Suggestion{
			{Message: "retry desktop connection"},
			{Message: "check that miopunch.exe is next to miopunch-desktop.exe"},
			{Message: "export runtime diagnostics"},
		}
	}

	return []poc.Suggestion{
		{Message: "retry desktop connection"},
		{Message: "check that ./miopunch is next to ./miopunch-desktop and executable"},
		{Message: "export runtime diagnostics"},
	}
}
