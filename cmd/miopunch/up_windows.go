//go:build windows

package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"time"

	"github.com/kardianos/service"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/task"
)

func runUp(args []string, stdout, stderr io.Writer) int {
	_ = stdout

	operatorSID, _, err := parseOperatorSID(args)
	if err != nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts: []poc.Fact{
				{Message: "invalid --operator_sid: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
		return int(poc.ExitCodeBadRequest)
	}
	if strings.TrimSpace(operatorSID) == "" {
		operatorSID, err = poc.CurrentOperatorSID()
		if err != nil {
			writeFailure(stderr, failureOutput{
				Stage:      "daemon",
				ReasonCode: poc.ReasonCodeInternal,
				ExitCode:   poc.ExitCodeInternal,
				Facts: []poc.Fact{
					{Message: "failed to determine operator SID: " + err.Error()},
				},
				Suggestions: []poc.Suggestion{
					{Message: "retry"},
				},
			})
			return int(poc.ExitCodeInternal)
		}
	}

	if !service.Interactive() {
		return runUpAsWindowsService(operatorSID, stderr)
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()

	return serveUpWindows(ctx, operatorSID, stderr)
}

type windowsUpProgram struct {
	operatorSID string
	stderr      io.Writer

	cancel context.CancelFunc
	done   chan struct{}
}

func (p *windowsUpProgram) Start(service.Service) error {
	ctx, cancel := context.WithCancel(context.Background())
	p.cancel = cancel
	p.done = make(chan struct{})

	go func() {
		defer close(p.done)
		_ = serveUpWindows(ctx, p.operatorSID, p.stderr)
	}()

	return nil
}

func (p *windowsUpProgram) Stop(service.Service) error {
	if p.cancel != nil {
		p.cancel()
	}
	if p.done == nil {
		return nil
	}
	select {
	case <-p.done:
	case <-time.After(5 * time.Second):
	}
	return nil
}

func runUpAsWindowsService(operatorSID string, stderr io.Writer) int {
	cfg := &service.Config{
		Name:        "miopunch",
		DisplayName: "miopunch",
		Description: "miopunch LocalAPI daemon (miopunch up)",
	}
	prg := &windowsUpProgram{
		operatorSID: strings.TrimSpace(operatorSID),
		stderr:      stderr,
	}
	svc, err := service.New(prg, cfg)
	if err != nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to start as windows service: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
		return int(poc.ExitCodeInternal)
	}
	if err := svc.Run(); err != nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "service runtime error: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
		return int(poc.ExitCodeInternal)
	}
	return 0
}

func serveUpWindows(ctx context.Context, operatorSID string, stderr io.Writer) int {
	systemAddr, err := localapi.DefaultSystemAddr(operatorSID)
	if err != nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts:      []poc.Fact{{Message: "failed to determine system localapi address"}},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
		return int(poc.ExitCodeInternal)
	}

	userAddr, _ := localapi.DefaultUserAddr(operatorSID)

	if err := probeLocalAPI(ctx, systemAddr); err == nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeConflict,
			ExitCode:   poc.ExitCodeConflict,
			Facts: []poc.Fact{
				{Message: "system localapi is reachable: " + systemAddr.String()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "stop the existing daemon before starting a new one"},
			},
		})
		return int(poc.ExitCodeConflict)
	} else if isPermissionError(err) {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeForbidden,
			ExitCode:   poc.ExitCodeForbidden,
			Facts: []poc.Fact{
				{Message: "permission denied probing system localapi: " + systemAddr.String()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "run from an elevated Administrator prompt"},
			},
		})
		return int(poc.ExitCodeForbidden)
	}

	if err := probeLocalAPI(ctx, userAddr); err == nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeConflict,
			ExitCode:   poc.ExitCodeConflict,
			Facts: []poc.Fact{
				{Message: "user localapi is reachable: " + userAddr.String()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "stop the existing daemon before starting a new one"},
			},
		})
		return int(poc.ExitCodeConflict)
	}

	ln, err := localapi.Listen(systemAddr, localapi.ListenModeSystem)
	if err != nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeUnavailable,
			ExitCode:   poc.ExitCodeUnavailable,
			Facts: []poc.Fact{
				{Message: "failed to listen: " + err.Error()},
				{Message: "addr=" + systemAddr.String()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry from an elevated Administrator prompt"},
			},
		})
		return int(poc.ExitCodeUnavailable)
	}
	defer func() { _ = ln.Close() }()

	mgr := task.NewManager()
	defer mgr.Close()

	api := localapi.NewServer(localapi.ListenModeSystem, mgr)
	httpServer := &http.Server{
		Handler: api.Handler(),
	}

	errCh := make(chan error, 1)
	go func() {
		errCh <- httpServer.Serve(ln)
	}()

	fmt.Fprintf(stderr, "miopunch up: serving LocalAPI (system) at %s\n", systemAddr.String())

	select {
	case <-ctx.Done():
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
		defer cancel()
		_ = httpServer.Shutdown(shutdownCtx)
		return 0
	case err := <-errCh:
		if errors.Is(err, http.ErrServerClosed) {
			return 0
		}
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "server error: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
		return int(poc.ExitCodeInternal)
	}
}

func probeLocalAPI(ctx context.Context, addr localapi.Addr) error {
	c, err := localapi.NewClient(addr)
	if err != nil {
		return err
	}
	return c.ProbeStatus(ctx)
}

func parseOperatorSID(args []string) (string, []string, error) {
	out := make([]string, 0, len(args))
	var operatorSID string
	for i := 0; i < len(args); i++ {
		a := args[i]
		if a == "--operator_sid" {
			if i+1 >= len(args) {
				return "", nil, errors.New("missing value")
			}
			operatorSID = strings.TrimSpace(args[i+1])
			i++
			continue
		}
		if strings.HasPrefix(a, "--operator_sid=") {
			operatorSID = strings.TrimSpace(strings.TrimPrefix(a, "--operator_sid="))
			continue
		}
		out = append(out, a)
	}
	return operatorSID, out, nil
}
