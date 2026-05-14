//go:build desktop

package main

import (
	"context"
	"fmt"
	"sync"

	"github.com/wailsapp/wails/v2/pkg/runtime"

	"github.com/miopunch/miopunch/internal/desktopbridge"
	"github.com/miopunch/miopunch/internal/localapi"
)

type App struct {
	mu sync.Mutex

	ctx context.Context

	overrideAddr  string
	client        *localapi.Client
	selectedAddr  localapi.Addr
	connState     desktopbridge.ConnectionState
	managedDaemon managedDaemon

	eventsCancel context.CancelFunc
	eventsDone   chan struct{}

	runtimeEventHook func(DesktopRuntimeEvent)

	termBridge *desktopbridge.TerminalWSBridge

	quitRequested bool
	tray          desktopTray
	closePrompt   func(context.Context) (closeChoice, error)
	hideWindow    func(context.Context)
	showWindow    func(context.Context)
	unminimise    func(context.Context)
	alwaysOnTop   func(context.Context, bool)
	quitRuntime   func(context.Context)
}

func NewApp() *App {
	return &App{tray: newPlatformTray()}
}

func (a *App) startup(ctx context.Context) {
	a.mu.Lock()
	a.ctx = ctx
	a.mu.Unlock()
}

func (a *App) domReady(ctx context.Context) {
	if err := a.ensureTerminalBridge(); err != nil && ctx != nil {
		runtime.EventsEmit(ctx, "desktop:startup_error", map[string]any{
			"component": "terminal_bridge",
			"error":     err,
		})
	}
}

func (a *App) shutdown(context.Context) {
	a.closeTray()
	a.stopEventsPump()
	a.closeTerminalBridge()
	a.stopManagedDaemon()
}

func (a *App) restoreWindow() {
	a.mu.Lock()
	ctx := a.ctx
	a.mu.Unlock()
	if ctx == nil {
		return
	}

	a.windowShow(ctx)
	a.windowUnminimise(ctx)
	a.windowSetAlwaysOnTop(ctx, true)
	a.windowSetAlwaysOnTop(ctx, false)
}

func (a *App) Quit() {
	a.mu.Lock()
	ctx := a.ctx
	a.quitRequested = true
	quitRuntime := a.quitRuntime
	a.mu.Unlock()
	if ctx == nil {
		return
	}
	if quitRuntime != nil {
		quitRuntime(ctx)
		return
	}
	runtime.Quit(ctx)
}

type managedDaemon interface {
	Stop(context.Context) error
}

type closeChoice int

const (
	closeChoiceQuit closeChoice = iota
	closeChoiceTray
)

func (a *App) beforeClose(ctx context.Context) bool {
	prevent, err := a.handleBeforeClose(ctx)
	if err != nil && ctx != nil {
		runtime.EventsEmit(ctx, "desktop:startup_error", map[string]any{
			"component": "window_lifecycle",
			"error":     err.Error(),
		})
	}
	return prevent
}

func (a *App) handleBeforeClose(ctx context.Context) (bool, error) {
	if a.isQuitRequested() {
		a.closeTray()
		return false, nil
	}

	choice, err := a.promptCloseChoice(ctx)
	if err != nil {
		a.markQuitRequested()
		return false, fmt.Errorf("prompt close choice: %w", err)
	}
	if choice == closeChoiceQuit {
		a.markQuitRequested()
		return false, nil
	}

	if err := a.showTray(); err != nil {
		a.markQuitRequested()
		return false, fmt.Errorf("show tray: %w", err)
	}
	a.windowHide(ctx)
	return true, nil
}

func (a *App) promptCloseChoice(ctx context.Context) (closeChoice, error) {
	a.mu.Lock()
	prompt := a.closePrompt
	a.mu.Unlock()
	if prompt != nil {
		return prompt(ctx)
	}

	resp, err := runtime.MessageDialog(ctx, runtime.MessageDialogOptions{
		Type:          runtime.QuestionDialog,
		Title:         "Close miopunch?",
		Message:       "Keep miopunch running in the system tray?",
		DefaultButton: "No",
	})
	if err != nil {
		return closeChoiceQuit, err
	}
	if resp == "Yes" {
		return closeChoiceTray, nil
	}
	return closeChoiceQuit, nil
}

func (a *App) windowHide(ctx context.Context) {
	a.mu.Lock()
	hideWindow := a.hideWindow
	a.mu.Unlock()
	if hideWindow != nil {
		hideWindow(ctx)
		return
	}
	runtime.WindowHide(ctx)
}

func (a *App) windowShow(ctx context.Context) {
	a.mu.Lock()
	showWindow := a.showWindow
	a.mu.Unlock()
	if showWindow != nil {
		showWindow(ctx)
		return
	}
	runtime.WindowShow(ctx)
}

func (a *App) windowUnminimise(ctx context.Context) {
	a.mu.Lock()
	unminimise := a.unminimise
	a.mu.Unlock()
	if unminimise != nil {
		unminimise(ctx)
		return
	}
	runtime.WindowUnminimise(ctx)
}

func (a *App) windowSetAlwaysOnTop(ctx context.Context, enabled bool) {
	a.mu.Lock()
	alwaysOnTop := a.alwaysOnTop
	a.mu.Unlock()
	if alwaysOnTop != nil {
		alwaysOnTop(ctx, enabled)
		return
	}
	runtime.WindowSetAlwaysOnTop(ctx, enabled)
}

func (a *App) showTray() error {
	a.mu.Lock()
	tray := a.tray
	a.mu.Unlock()
	if tray == nil {
		return fmt.Errorf("desktop tray is unavailable")
	}
	return tray.Show(a.restoreWindow, a.Quit)
}

func (a *App) closeTray() {
	a.mu.Lock()
	tray := a.tray
	a.mu.Unlock()
	if tray == nil {
		return
	}
	tray.Close()
}

func (a *App) markQuitRequested() {
	a.mu.Lock()
	a.quitRequested = true
	a.mu.Unlock()
}

func (a *App) isQuitRequested() bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.quitRequested
}
