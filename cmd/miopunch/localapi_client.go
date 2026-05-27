package main

import (
	"context"
	"fmt"
	"io"
	"os"
	"os/exec"
	"runtime"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
)

const defaultDaemonBootstrapTimeout = 10 * time.Second

type localAPIConnectionError struct {
	Failure failureOutput
}

func (e *localAPIConnectionError) Error() string {
	return string(e.Failure.ReasonCode)
}

type localAPIConnectorDeps struct {
	currentOperatorSID func() (string, error)
	defaultSystemAddr  func(string) (localapi.Addr, error)
	defaultUserAddr    func(string) (localapi.Addr, error)
	probe              func(context.Context, localapi.Addr) (*localapi.Client, error)
	bootstrap          func(localapi.Addr) error
}

func connectLocalAPI(ctx context.Context, override string) (*localapi.Client, localapi.Addr, error) {
	return connectLocalAPIWithDeps(ctx, override, localAPIConnectorDeps{
		currentOperatorSID: poc.CurrentOperatorSID,
		defaultSystemAddr:  localapi.DefaultSystemAddr,
		defaultUserAddr:    localapi.DefaultUserAddr,
		probe:              probeLocalAPIClient,
		bootstrap:          bootstrapDaemonAndWait,
	})
}

func connectLocalAPIWithDeps(ctx context.Context, override string, deps localAPIConnectorDeps) (*localapi.Client, localapi.Addr, error) {
	if ctx == nil {
		ctx = context.Background()
	}
	deps = deps.withDefaults()

	if strings.TrimSpace(override) != "" {
		addr, err := localapi.ParseAddr(override)
		if err != nil {
			return nil, localapi.Addr{}, &localAPIConnectionError{
				Failure: failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts: []poc.Fact{
						{Message: "invalid --localapi: " + strings.TrimSpace(override)},
					},
					Suggestions: []poc.Suggestion{
						{Message: "use: --localapi unix:/path/to/localapi.sock"},
						{Message: `or: --localapi npipe:\\.\pipe\miopunch\localapi-<operator_sid>`},
					},
				},
			}
		}
		client, err := deps.probe(ctx, addr)
		if err != nil {
			return nil, localapi.Addr{}, unreachableLocalAPIFailure(addr, err)
		}
		return client, addr, nil
	}

	operatorSID, err := deps.currentOperatorSID()
	if err != nil {
		return nil, localapi.Addr{}, &localAPIConnectionError{
			Failure: failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeInternal,
				ExitCode:   poc.ExitCodeInternal,
				Facts: []poc.Fact{
					{Message: "failed to determine operator SID: " + err.Error()},
				},
				Suggestions: []poc.Suggestion{
					{Message: "retry"},
				},
			},
		}
	}

	systemAddr, systemAddrErr := deps.defaultSystemAddr(operatorSID)
	userAddr, userAddrErr := deps.defaultUserAddr(operatorSID)

	primaryAddr, primaryName := choosePrimaryLocalAPI(systemAddr, systemAddrErr, userAddr, userAddrErr)
	if primaryName != "" {
		client, err := deps.probe(ctx, primaryAddr)
		if err == nil {
			return client, primaryAddr, nil
		}
		if isPermissionError(err) {
			return nil, localapi.Addr{}, permissionLocalAPIFailure(primaryName, primaryAddr)
		}
	}

	if primaryName == "user" && systemAddrErr == nil {
		client, err := deps.probe(ctx, systemAddr)
		if err == nil {
			return client, systemAddr, nil
		}
		if isPermissionError(err) {
			return nil, localapi.Addr{}, permissionLocalAPIFailure("system", systemAddr)
		}
	}

	facts := localAPIResolutionFacts(systemAddr, systemAddrErr, userAddr, userAddrErr)
	if primaryName != "" {
		bootstrapErr := deps.bootstrap(primaryAddr)
		if bootstrapErr == nil {
			client, err := deps.probe(ctx, primaryAddr)
			if err == nil {
				return client, primaryAddr, nil
			}
			if isPermissionError(err) {
				return nil, localapi.Addr{}, permissionLocalAPIFailure(primaryName, primaryAddr)
			}
			bootstrapErr = err
		}
		facts = append(facts,
			poc.Fact{Message: "bootstrap_addr=" + primaryAddr.String()},
			poc.Fact{Message: "bootstrap_error=" + bootstrapErr.Error()},
		)
	}

	return nil, localapi.Addr{}, &localAPIConnectionError{
		Failure: failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeDaemonNotRunning,
			ExitCode:   poc.ExitCodeUnavailable,
			Facts:      facts,
			Suggestions: []poc.Suggestion{
				{Message: "start the daemon: miopunch up"},
			},
		},
	}
}

func (deps localAPIConnectorDeps) withDefaults() localAPIConnectorDeps {
	if deps.currentOperatorSID == nil {
		deps.currentOperatorSID = poc.CurrentOperatorSID
	}
	if deps.defaultSystemAddr == nil {
		deps.defaultSystemAddr = localapi.DefaultSystemAddr
	}
	if deps.defaultUserAddr == nil {
		deps.defaultUserAddr = localapi.DefaultUserAddr
	}
	if deps.probe == nil {
		deps.probe = probeLocalAPIClient
	}
	if deps.bootstrap == nil {
		deps.bootstrap = bootstrapDaemonAndWait
	}
	return deps
}

func localAPIResolutionFacts(
	systemAddr localapi.Addr,
	systemAddrErr error,
	userAddr localapi.Addr,
	userAddrErr error,
) []poc.Fact {
	facts := []poc.Fact{}
	if systemAddrErr == nil {
		facts = append(facts, poc.Fact{Message: "system_addr=" + systemAddr.String()})
	} else {
		facts = append(facts, poc.Fact{Message: "system_addr_error=" + systemAddrErr.Error()})
	}
	if userAddrErr == nil {
		facts = append(facts, poc.Fact{Message: "user_addr=" + userAddr.String()})
	} else {
		facts = append(facts, poc.Fact{Message: "user_addr_error=" + userAddrErr.Error()})
	}
	return facts
}

func choosePrimaryLocalAPI(
	systemAddr localapi.Addr,
	systemAddrErr error,
	userAddr localapi.Addr,
	userAddrErr error,
) (localapi.Addr, string) {
	if isRootOperator() {
		if systemAddrErr == nil {
			return systemAddr, "system"
		}
		if userAddrErr == nil {
			return userAddr, "user"
		}
		return localapi.Addr{}, ""
	}
	if userAddrErr == nil {
		return userAddr, "user"
	}
	if systemAddrErr == nil {
		return systemAddr, "system"
	}
	return localapi.Addr{}, ""
}

func bootstrapDaemonAndWait(addr localapi.Addr) error {
	exe, err := os.Executable()
	if err != nil {
		return err
	}

	cmd := exec.Command(exe, "up")
	cmd.Stdout = io.Discard
	cmd.Stderr = io.Discard
	configureBootstrapCommand(cmd)
	if err := cmd.Start(); err != nil {
		return err
	}

	deadline := time.Now().Add(defaultDaemonBootstrapTimeout)
	for time.Now().Before(deadline) {
		client, err := probeLocalAPIClient(context.Background(), addr)
		if err == nil {
			return client.ProbeStatus(context.Background())
		}
		time.Sleep(100 * time.Millisecond)
	}
	return fmt.Errorf("timed out waiting for localapi at %s", addr.String())
}

func probeLocalAPIClient(ctx context.Context, addr localapi.Addr) (*localapi.Client, error) {
	client, err := localapi.NewClient(addr)
	if err != nil {
		return nil, err
	}
	if err := client.ProbeStatus(ctx); err != nil {
		return nil, err
	}
	return client, nil
}

func unreachableLocalAPIFailure(addr localapi.Addr, err error) error {
	if isPermissionError(err) {
		return permissionLocalAPIFailure("override", addr)
	}
	return &localAPIConnectionError{
		Failure: failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeDaemonNotRunning,
			ExitCode:   poc.ExitCodeUnavailable,
			Facts: []poc.Fact{
				{Message: "localapi is not reachable"},
				{Message: "addr=" + addr.String()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "start the daemon: miopunch up"},
			},
		},
	}
}

func permissionLocalAPIFailure(endpoint string, addr localapi.Addr) error {
	facts := []poc.Fact{
		{Message: "permission denied connecting to localapi"},
		{Message: "endpoint=" + endpoint},
		{Message: "addr=" + addr.String()},
	}
	suggestions := userLocalAPIPermissionSuggestions()
	if endpoint == "system" {
		suggestions = systemLocalAPIPermissionSuggestions()
	}
	return &localAPIConnectionError{
		Failure: failureOutput{
			Stage:       "cli",
			ReasonCode:  poc.ReasonCodeForbidden,
			ExitCode:    poc.ExitCodeForbidden,
			Facts:       facts,
			Suggestions: suggestions,
		},
	}
}

func systemLocalAPIPermissionSuggestions() []poc.Suggestion {
	if runtime.GOOS == "windows" {
		return []poc.Suggestion{
			{Message: "run the CLI as the same Windows user that installed the service"},
			{Message: "or reinstall the system service as the intended operator"},
		}
	}

	return []poc.Suggestion{
		{Message: "join the operator group: " + poc.LinuxOperatorGroup},
		{Message: "or run: sudo miopunch <cmd>"},
	}
}

func userLocalAPIPermissionSuggestions() []poc.Suggestion {
	if runtime.GOOS == "windows" {
		return []poc.Suggestion{
			{Message: "run the CLI as the same Windows user that owns the localapi pipe"},
			{Message: "or check pipe permissions and retry"},
		}
	}

	return []poc.Suggestion{
		{Message: "check socket permissions"},
	}
}
