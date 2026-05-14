//go:build windows

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
	"time"

	"github.com/kardianos/service"

	"github.com/miopunch/miopunch/internal/http_panel"
	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocacceptor"
	"github.com/miopunch/miopunch/internal/task"
)

func runUp(globalOpt globalOptions, args []string, stdout, stderr io.Writer) int {
	_ = stdout
	initDaemonLogger()

	operatorSID, rest, err := parseOperatorSID(args)
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

	upOpt, _, err := parseUpOptions(rest)
	if err != nil {
		writeFailure(stderr, failureOutput{
			Stage:      "daemon",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts: []poc.Fact{
				{Message: err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry with valid flags"},
			},
		})
		return int(poc.ExitCodeBadRequest)
	}
	if strings.TrimSpace(globalOpt.LocalAPIOverride) != "" {
		upOpt.LocalAPIOverride = strings.TrimSpace(globalOpt.LocalAPIOverride)
	}
	upOpt, err = applySessionStatePath(upOpt)
	if err != nil {
		writeFailure(stderr, sessionStatePathFailure(err))
		return int(poc.ExitCodeUnavailable)
	}
	logDaemonStatePath(upOpt.StatePath)
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

	if !upOpt.Session && !service.Interactive() {
		return runUpAsWindowsService(operatorSID, upOpt, stderr)
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()

	return serveUpWindows(ctx, operatorSID, upOpt, localapi.ListenModeUser, stderr)
}

type windowsUpProgram struct {
	operatorSID string
	upOpt       upOptions
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
		_ = serveUpWindows(ctx, p.operatorSID, p.upOpt, localapi.ListenModeSystem, p.stderr)
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

func runUpAsWindowsService(operatorSID string, upOpt upOptions, stderr io.Writer) int {
	cfg := &service.Config{
		Name:        "miopunch",
		DisplayName: "miopunch",
		Description: "miopunch LocalAPI daemon (miopunch up)",
	}
	prg := &windowsUpProgram{
		operatorSID: strings.TrimSpace(operatorSID),
		upOpt:       upOpt,
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

func serveUpWindows(ctx context.Context, operatorSID string, upOpt upOptions, mode localapi.ListenMode, stderr io.Writer) int {
	override := strings.TrimSpace(upOpt.LocalAPIOverride)
	var overrideAddr localapi.Addr
	var err error
	if override != "" {
		overrideAddr, err = localapi.ParseAddr(override)
		if err != nil {
			writeFailure(stderr, failureOutput{
				Stage:      "daemon",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Facts: []poc.Fact{
					{Message: "invalid --localapi: " + err.Error()},
				},
				Suggestions: []poc.Suggestion{
					{Message: "retry with a localapi address"},
				},
			})
			return int(poc.ExitCodeBadRequest)
		}
	}

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

	if override == "" {
		if mode == localapi.ListenModeSystem {
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
		} else if isPermissionError(err) {
			writeFailure(stderr, failureOutput{
				Stage:      "daemon",
				ReasonCode: poc.ReasonCodeForbidden,
				ExitCode:   poc.ExitCodeForbidden,
				Facts: []poc.Fact{
					{Message: "permission denied probing user localapi: " + userAddr.String()},
				},
				Suggestions: []poc.Suggestion{
					{Message: "check pipe permissions and retry"},
				},
			})
			return int(poc.ExitCodeForbidden)
		}
	}

	addr := userAddr
	if override != "" {
		addr = overrideAddr
	} else if mode == localapi.ListenModeSystem {
		addr = systemAddr
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
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry from an elevated Administrator prompt"},
			},
		})
		return int(poc.ExitCodeUnavailable)
	}
	defer func() { _ = ln.Close() }()

	var mgr *task.Manager
	if strings.TrimSpace(upOpt.StatePath) != "" {
		mgr = task.NewManagerWithStatePath(upOpt.StatePath)
	} else {
		mgr = task.NewManager()
	}
	defer mgr.Close()

	var panel *http_panel.Server
	var panelLn net.Listener
	if upOpt.HTTPPanel {
		var listenErr error
		panelLn, _, listenErr = http_panel.Listen(upOpt.HTTPPanelListenAddr)
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
				Facts: []poc.Fact{
					{Message: "failed to listen http panel: " + listenErr.Error()},
				},
				Suggestions: []poc.Suggestion{
					{Message: "change the port via --http_panel_listen_addr and retry"},
				},
			})
			return int(poc.ExitCodeUnavailable)
		}
		panel = http_panel.NewServer(panelLn.Addr().String(), mgr)
		defer func() { _ = panelLn.Close() }()
	}

	api := localapi.NewServer(mode, mgr)
	httpServer := &http.Server{
		Handler: api.Handler(),
	}

	panelHTTPServer := &http.Server{}
	errCh := make(chan error, 2)
	go func() {
		errCh <- fmt.Errorf("localapi serve: %w", httpServer.Serve(ln))
	}()
	if panel != nil && panelLn != nil {
		panelHTTPServer.Handler = panel.Handler()
		go func() {
			errCh <- fmt.Errorf("http panel serve: %w", panelHTTPServer.Serve(panelLn))
		}()
	}
	go func() {
		_ = pocacceptor.Run(ctx, pocacceptor.Config{StatePath: upOpt.StatePath, RuntimeEvidence: mgr})
	}()

	fmt.Fprintf(stderr, "miopunch up: serving LocalAPI (%s) at %s\n", mode, addr.String())
	if panel != nil {
		fmt.Fprintf(stderr, "miopunch up: serving HTTP panel at %s/\n", panel.Origin())
	}

	select {
	case <-ctx.Done():
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
		defer cancel()
		_ = httpServer.Shutdown(shutdownCtx)
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
