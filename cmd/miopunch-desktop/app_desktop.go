//go:build desktop

package main

import (
	"context"
	"sync"

	"github.com/wailsapp/wails/v2/pkg/runtime"

	"github.com/miopunch/miopunch/internal/desktopbridge"
	"github.com/miopunch/miopunch/internal/localapi"
)

type App struct {
	mu sync.Mutex

	ctx context.Context

	overrideAddr string
	client       *localapi.Client
	selectedAddr localapi.Addr
	connState    desktopbridge.ConnectionState

	eventsCancel context.CancelFunc
	eventsDone   chan struct{}

	termBridge *desktopbridge.TerminalWSBridge
}

func NewApp() *App {
	return &App{}
}

func (a *App) startup(ctx context.Context) {
	a.mu.Lock()
	a.ctx = ctx
	a.mu.Unlock()

	if err := a.ensureTerminalBridge(); err != nil && ctx != nil {
		runtime.EventsEmit(ctx, "desktop:startup_error", map[string]any{
			"component": "terminal_bridge",
			"error":     err,
		})
	}
	_ = a.Connect()
}

func (a *App) shutdown(context.Context) {
	a.stopEventsPump()
	a.closeTerminalBridge()
}
