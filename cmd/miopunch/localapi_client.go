package main

import (
	"context"
	"fmt"
	"runtime"
	"strings"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
)

type localAPIConnectionError struct {
	Failure failureOutput
}

func (e *localAPIConnectionError) Error() string {
	return string(e.Failure.ReasonCode)
}

func connectLocalAPI(ctx context.Context, override string) (*localapi.Client, localapi.Addr, error) {
	if strings.TrimSpace(override) != "" {
		addr, err := parseLocalAPIAddr(override)
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
		c, err := localapi.NewClient(addr)
		if err != nil {
			return nil, localapi.Addr{}, err
		}
		if err := c.ProbeStatus(ctx); err != nil {
			if isPermissionError(err) {
				return nil, localapi.Addr{}, &localAPIConnectionError{
					Failure: failureOutput{
						Stage:      "cli",
						ReasonCode: poc.ReasonCodeForbidden,
						ExitCode:   poc.ExitCodeForbidden,
						Facts: []poc.Fact{
							{Message: "permission denied connecting to localapi"},
							{Message: "addr=" + addr.String()},
						},
						Suggestions: []poc.Suggestion{
							{Message: "check operator permissions"},
						},
					},
				}
			}

			return nil, localapi.Addr{}, &localAPIConnectionError{
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
		return c, addr, nil
	}

	operatorSID, err := poc.CurrentOperatorSID()
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

	systemAddr, err := localapi.DefaultSystemAddr(operatorSID)
	if err == nil {
		c, err := localapi.NewClient(systemAddr)
		if err != nil {
			return nil, localapi.Addr{}, err
		}
		if err := c.ProbeStatus(ctx); err == nil {
			return c, systemAddr, nil
		} else if isPermissionError(err) {
			return nil, localapi.Addr{}, &localAPIConnectionError{
				Failure: failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeForbidden,
					ExitCode:   poc.ExitCodeForbidden,
					Facts: []poc.Fact{
						{Message: "permission denied connecting to system localapi"},
						{Message: "addr=" + systemAddr.String()},
					},
					Suggestions: systemLocalAPIPermissionSuggestions(),
				},
			}
		}
	}

	userAddr, userAddrErr := localapi.DefaultUserAddr(operatorSID)
	if userAddrErr == nil {
		c, err := localapi.NewClient(userAddr)
		if err != nil {
			return nil, localapi.Addr{}, err
		}
		if err := c.ProbeStatus(ctx); err == nil {
			return c, userAddr, nil
		} else if isPermissionError(err) {
			return nil, localapi.Addr{}, &localAPIConnectionError{
				Failure: failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeForbidden,
					ExitCode:   poc.ExitCodeForbidden,
					Facts: []poc.Fact{
						{Message: "permission denied connecting to user localapi"},
						{Message: "addr=" + userAddr.String()},
					},
					Suggestions: userLocalAPIPermissionSuggestions(),
				},
			}
		}
	}

	facts := []poc.Fact{}
	if err == nil {
		facts = append(facts, poc.Fact{Message: "system_addr=" + systemAddr.String()})
	}
	if userAddrErr == nil {
		facts = append(facts, poc.Fact{Message: "user_addr=" + userAddr.String()})
	}
	if userAddrErr != nil {
		facts = append(facts, poc.Fact{Message: "user_addr_error=" + userAddrErr.Error()})
	}

	return nil, localapi.Addr{}, &localAPIConnectionError{
		Failure: failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeDaemonNotRunning,
			ExitCode:   poc.ExitCodeUnavailable,
			Facts:      facts,
			Suggestions: []poc.Suggestion{
				{Message: "start the daemon: miopunch up"},
				{Message: "or install system service: miopunch install-system-daemon"},
			},
		},
	}
}

func parseLocalAPIAddr(value string) (localapi.Addr, error) {
	v := strings.TrimSpace(value)
	switch {
	case strings.HasPrefix(v, "unix:"):
		path := strings.TrimSpace(strings.TrimPrefix(v, "unix:"))
		if path == "" {
			return localapi.Addr{}, fmt.Errorf("empty unix socket path")
		}
		return localapi.Addr{Transport: localapi.TransportUnix, Path: path}, nil
	case strings.HasPrefix(v, "npipe:"):
		path := strings.TrimSpace(strings.TrimPrefix(v, "npipe:"))
		if path == "" {
			return localapi.Addr{}, fmt.Errorf("empty npipe path")
		}
		return localapi.Addr{Transport: localapi.TransportNpipe, Path: path}, nil
	default:
		return localapi.Addr{}, fmt.Errorf("unsupported addr format: %q", value)
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
