//go:build !windows

package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/miopunch/miopunch/internal/bundlepath"
	"github.com/miopunch/miopunch/internal/http_panel"
	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
)

func runUp(globalOpt globalOptions, args []string, stdout, stderr io.Writer) int {
	_ = stdout

	opt, _, err := parseUpOptions(args)
	if err != nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts:      []poc.Fact{{Message: err.Error()}},
			Suggestions: []poc.Suggestion{
				{Message: "retry with valid flags"},
			},
		})
		return int(poc.ExitCodeBadRequest)
	}
	if strings.TrimSpace(globalOpt.LocalAPIOverride) != "" {
		opt.LocalAPIOverride = strings.TrimSpace(globalOpt.LocalAPIOverride)
	}
	initDaemonLogger(opt.LogLevel)
	opt, err = applySessionStatePath(opt)
	if err != nil {
		writeFailure(stderr, sessionStatePathFailure(err))
		return int(poc.ExitCodeUnavailable)
	}
	logDaemonStatePath(opt.StatePath)

	runtimeRoot, err := resolveRuntimeRoot(opt.StatePath)
	if err != nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeUnavailable,
			ExitCode:   poc.ExitCodeUnavailable,
			Facts:      []poc.Fact{{Message: "failed to resolve runtime root: " + err.Error()}},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
		return int(poc.ExitCodeUnavailable)
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	override := strings.TrimSpace(opt.LocalAPIOverride)
	var overrideAddr localapi.Addr
	if override != "" {
		overrideAddr, err = localapi.ParseAddr(override)
		if err != nil {
			writeFailure(stderr, failureOutput{
				Stage:      "daemon",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Facts:      []poc.Fact{{Message: "invalid --localapi: " + err.Error()}},
				Suggestions: []poc.Suggestion{
					{Message: "retry with a unix: localapi address"},
				},
			})
			return int(poc.ExitCodeBadRequest)
		}
	}

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

	if override == "" {
		if failure, ok := defaultLocalAPIConflict(ctx, systemAddr, userAddr, userAddrErr, stderr); ok {
			writeFailure(stderr, failure)
			return int(failure.ExitCode)
		}
	}

	mode := localapi.ListenModeUser
	addr := userAddr
	if override != "" {
		addr = overrideAddr
	} else if opt.Session {
		sessionAddr, sessionErr := sessionLocalAPIAddr()
		if sessionErr != nil {
			writeFailure(stderr, failureOutput{
				Stage:      "daemon",
				ReasonCode: poc.ReasonCodeUnavailable,
				ExitCode:   poc.ExitCodeUnavailable,
				Facts: []poc.Fact{
					{Message: "failed to resolve portable localapi address: " + sessionErr.Error()},
				},
				Suggestions: []poc.Suggestion{
					{Message: "extract the session bundle into a writable directory and retry"},
					{Message: "or pass --localapi unix:/path/to/localapi.sock explicitly"},
				},
			})
			return int(poc.ExitCodeUnavailable)
		}
		addr = sessionAddr
	} else if isRootOperator() {
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

	runtimeInstance, err := pocruntime.Open(pocruntime.Options{
		Root:      runtimeRoot,
		BrokerURL: strings.TrimSpace(opt.BrokerOverride),
	})
	if err != nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeUnavailable,
			ExitCode:   poc.ExitCodeUnavailable,
			Facts: []poc.Fact{
				{Message: "failed to open runtime: " + err.Error()},
				{Message: "root=" + runtimeRoot},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
		return int(poc.ExitCodeUnavailable)
	}
	defer func() { _ = runtimeInstance.Close() }()

	api := localapi.NewServer(mode, runtimeInstance)
	defer func() { _ = api.Close() }()

	var panel *http_panel.Server
	var panelLn net.Listener
	panelHTTPServer := &http.Server{}
	if opt.HTTPPanel {
		var listenErr error
		panelLn, _, listenErr = http_panel.Listen(opt.HTTPPanelListenAddr)
		if listenErr != nil {
			var addrErr *http_panel.ListenAddrError
			if errors.As(listenErr, &addrErr) {
				writeFailure(stderr, failureOutput{
					Stage:      "daemon",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts: []poc.Fact{
						{Message: "http panel is loopback-only (127.0.0.1)"},
						{Message: "listen_addr=" + addrErr.ListenAddr},
						{Message: "error=" + addrErr.Problem},
					},
					Suggestions: []poc.Suggestion{
						{Message: "use: --http_panel_listen_addr " + http_panel.DefaultListenAddr},
					},
				})
				return int(poc.ExitCodeBadRequest)
			}

			writeFailure(stderr, failureOutput{
				Stage:      "daemon",
				ReasonCode: poc.ReasonCodeUnavailable,
				ExitCode:   poc.ExitCodeUnavailable,
				Facts:      []poc.Fact{{Message: "failed to listen http panel: " + listenErr.Error()}},
				Suggestions: []poc.Suggestion{
					{Message: "change the port via --http_panel_listen_addr and retry"},
				},
			})
			return int(poc.ExitCodeUnavailable)
		}
		panel = http_panel.NewServer(panelLn.Addr().String(), addr)
		panelHTTPServer.Handler = panel.Handler()
		defer func() { _ = panelLn.Close() }()
	}

	errCh := make(chan error, 2)
	go func() {
		errCh <- fmt.Errorf("localapi serve: %w", api.Serve(ln))
	}()
	if panel != nil && panelLn != nil {
		go func() {
			errCh <- fmt.Errorf("http panel serve: %w", panelHTTPServer.Serve(panelLn))
		}()
	}

	fmt.Fprintf(stderr, "miopunch up: serving LocalAPI (%s) at %s\n", mode, addr.String())
	fmt.Fprintf(stderr, "miopunch up: runtime root %s\n", runtimeRoot)
	if panel != nil {
		fmt.Fprintf(stderr, "miopunch up: serving HTTP panel at %s/\n", panel.Origin())
	}

	select {
	case <-ctx.Done():
		_ = ln.Close()
		_ = api.Close()
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
		defer cancel()
		_ = panelHTTPServer.Shutdown(shutdownCtx)
		return 0
	case err := <-errCh:
		if errors.Is(err, http.ErrServerClosed) {
			return 0
		}
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts:      []poc.Fact{{Message: "server error: " + err.Error()}},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
		return int(poc.ExitCodeInternal)
	}
}

func sessionLocalAPIAddr() (localapi.Addr, error) {
	path, err := bundlepath.LocalAPIPath()
	if err != nil {
		return localapi.Addr{}, err
	}
	return localapi.Addr{
		Transport: localapi.TransportUnix,
		Path:      path,
	}, nil
}

func defaultLocalAPIConflict(ctx context.Context, systemAddr, userAddr localapi.Addr, userAddrErr error, stderr io.Writer) (failureOutput, bool) {
	if err := probeLocalAPI(ctx, systemAddr); err == nil {
		return failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeConflict,
			ExitCode:   poc.ExitCodeConflict,
			Facts: []poc.Fact{
				{Message: "system localapi is reachable: " + systemAddr.String()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "stop the existing daemon before starting a new one"},
			},
		}, true
	} else if isPermissionError(err) {
		fmt.Fprintf(stderr, "miopunch up: permission denied probing system LocalAPI; continuing with user session LocalAPI (%s)\n", systemAddr.String())
	}

	if userAddrErr != nil {
		return failureOutput{}, false
	}
	if err := probeLocalAPI(ctx, userAddr); err == nil {
		return failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeConflict,
			ExitCode:   poc.ExitCodeConflict,
			Facts: []poc.Fact{
				{Message: "user localapi is reachable: " + userAddr.String()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "stop the existing daemon before starting a new one"},
			},
		}, true
	} else if isPermissionError(err) {
		return failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeForbidden,
			ExitCode:   poc.ExitCodeForbidden,
			Facts: []poc.Fact{
				{Message: "permission denied probing user localapi: " + userAddr.String()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "check socket permissions"},
			},
		}, true
	}

	return failureOutput{}, false
}

var probeLocalAPI = realProbeLocalAPI

func realProbeLocalAPI(ctx context.Context, addr localapi.Addr) error {
	client, err := localapi.NewClient(addr)
	if err != nil {
		return err
	}
	return client.ProbeStatus(ctx)
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
