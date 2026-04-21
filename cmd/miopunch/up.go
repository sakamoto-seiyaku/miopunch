//go:build !windows

package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/task"
)

func runUp(args []string, stdout, stderr io.Writer) int {
	_ = args

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	systemAddr, err := localapi.DefaultSystemAddr("")
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

	userAddr, userAddrErr := localapi.DefaultUserAddr("")

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
				{Message: "join the operator group: " + poc.LinuxOperatorGroup},
				{Message: "or run: sudo miopunch up"},
			},
		})
		return int(poc.ExitCodeForbidden)
	}

	if userAddrErr == nil {
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
		} else if isPermissionError(err) {
			writeFailure(stderr, failureOutput{
				Stage:      "daemon",
				ReasonCode: poc.ReasonCodeForbidden,
				ExitCode:   poc.ExitCodeForbidden,
				Facts: []poc.Fact{
					{Message: "permission denied probing user localapi: " + userAddr.String()},
				},
				Suggestions: []poc.Suggestion{
					{Message: "check socket permissions"},
				},
			})
			return int(poc.ExitCodeForbidden)
		}
	}

	mode := localapi.ListenModeUser
	addr := userAddr
	if os.Geteuid() == 0 {
		mode = localapi.ListenModeSystem
		addr = systemAddr
	} else if userAddrErr != nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts: []poc.Fact{
				{Message: "failed to determine user localapi address: " + userAddrErr.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "set XDG_RUNTIME_DIR and retry"},
				{Message: "or run: sudo miopunch up"},
			},
		})
		return int(poc.ExitCodeBadRequest)
	}

	if err := cleanupStaleLocalAPI(ctx, addr); err != nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to cleanup localapi address: " + err.Error()},
				{Message: "addr=" + addr.String()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
		return int(poc.ExitCodeInternal)
	}

	ln, err := localapi.Listen(addr, mode)
	if err != nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeUnavailable,
			ExitCode:   poc.ExitCodeUnavailable,
			Facts: []poc.Fact{
				{Message: "failed to listen: " + err.Error()},
				{Message: "addr=" + addr.String()},
				{Message: "mode=" + string(mode)},
			},
			Suggestions: []poc.Suggestion{
				{Message: "check permissions and retry"},
			},
		})
		return int(poc.ExitCodeUnavailable)
	}
	defer func() { _ = ln.Close() }()

	mgr := task.NewManager()
	defer mgr.Close()

	api := localapi.NewServer(mode, mgr)
	httpServer := &http.Server{
		Handler: api.Handler(),
	}

	errCh := make(chan error, 1)
	go func() {
		errCh <- httpServer.Serve(ln)
	}()

	fmt.Fprintf(stderr, "miopunch up: serving LocalAPI (%s) at %s\n", mode, addr.String())

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

func cleanupStaleLocalAPI(ctx context.Context, addr localapi.Addr) error {
	switch addr.Transport {
	case localapi.TransportUnix:
		return cleanupStaleUnixSocket(ctx, addr.Path)
	default:
		return nil
	}
}

func cleanupStaleUnixSocket(ctx context.Context, path string) error {
	_, statErr := os.Stat(path)
	if statErr != nil {
		if errors.Is(statErr, os.ErrNotExist) {
			return nil
		}
		return statErr
	}

	// If the socket is reachable, keep it.
	addr := localapi.Addr{Transport: localapi.TransportUnix, Path: path}
	err := probeLocalAPI(ctx, addr)
	if err == nil {
		return fmt.Errorf("localapi already running at %s", addr.String())
	}
	if isPermissionError(err) {
		return err
	}

	return os.Remove(path)
}

func isPermissionError(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, os.ErrPermission) {
		return true
	}
	return errors.Is(err, syscall.EACCES) || errors.Is(err, syscall.EPERM)
}
