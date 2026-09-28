//go:build desktop

package main

import (
	"context"
	"errors"
	"testing"
)

func TestHandleBeforeCloseExplicitQuitAllowsClose(t *testing.T) {
	app := NewApp()
	app.quitRequested = true
	app.closePrompt = func(context.Context) (closeChoice, error) {
		t.Fatalf("handleBeforeClose() prompted despite explicit quit")
		return closeChoiceQuit, nil
	}

	prevent, err := app.handleBeforeClose(context.Background())
	if err != nil {
		t.Fatalf("handleBeforeClose() error = %v, want nil", err)
	}
	if prevent {
		t.Fatalf("handleBeforeClose() prevent = true, want false")
	}
}

func TestHandleBeforeCloseTrayChoicePreventsCloseAndHidesWindow(t *testing.T) {
	tray := &fakeTray{}
	app := NewApp()
	app.tray = tray
	app.closePrompt = func(context.Context) (closeChoice, error) {
		return closeChoiceTray, nil
	}
	hideCalls := 0
	app.hideWindow = func(context.Context) {
		hideCalls++
	}

	prevent, err := app.handleBeforeClose(context.Background())
	if err != nil {
		t.Fatalf("handleBeforeClose() error = %v, want nil", err)
	}
	if !prevent {
		t.Fatalf("handleBeforeClose() prevent = false, want true")
	}
	if tray.showCalls != 1 {
		t.Fatalf("fakeTray.Show calls = %d, want 1", tray.showCalls)
	}
	if hideCalls != 1 {
		t.Fatalf("hideWindow calls = %d, want 1", hideCalls)
	}
	if app.isQuitRequested() {
		t.Fatalf("isQuitRequested() = true, want false")
	}
}

func TestHandleBeforeCloseQuitChoiceAllowsShutdown(t *testing.T) {
	tray := &fakeTray{}
	app := NewApp()
	app.tray = tray
	app.closePrompt = func(context.Context) (closeChoice, error) {
		return closeChoiceQuit, nil
	}
	app.hideWindow = func(context.Context) {
		t.Fatalf("handleBeforeClose() hid window for quit choice")
	}

	prevent, err := app.handleBeforeClose(context.Background())
	if err != nil {
		t.Fatalf("handleBeforeClose() error = %v, want nil", err)
	}
	if prevent {
		t.Fatalf("handleBeforeClose() prevent = true, want false")
	}
	if tray.showCalls != 0 {
		t.Fatalf("fakeTray.Show calls = %d, want 0", tray.showCalls)
	}
	if !app.isQuitRequested() {
		t.Fatalf("isQuitRequested() = false, want true")
	}
}

func TestHandleBeforeClosePromptFailureFallsBackToQuit(t *testing.T) {
	wantErr := errors.New("dialog failed")
	app := NewApp()
	app.closePrompt = func(context.Context) (closeChoice, error) {
		return closeChoiceQuit, wantErr
	}

	prevent, err := app.handleBeforeClose(context.Background())
	if err == nil {
		t.Fatalf("handleBeforeClose() error = nil, want error")
	}
	if prevent {
		t.Fatalf("handleBeforeClose() prevent = true, want false")
	}
	if !app.isQuitRequested() {
		t.Fatalf("isQuitRequested() = false, want true")
	}
}

func TestQuitMarksExplicitQuitAndCallsRuntime(t *testing.T) {
	app := NewApp()
	app.startup(context.Background())
	quitCalls := 0
	app.quitRuntime = func(context.Context) {
		quitCalls++
	}

	app.Quit()

	if quitCalls != 1 {
		t.Fatalf("Quit() runtime calls = %d, want 1", quitCalls)
	}
	if !app.isQuitRequested() {
		t.Fatalf("isQuitRequested() = false, want true")
	}
}

func TestRestoreWindowShowsRestoresAndRaisesWindow(t *testing.T) {
	app := NewApp()
	app.startup(context.Background())

	var calls []string
	app.showWindow = func(context.Context) {
		calls = append(calls, "show")
	}
	app.unminimise = func(context.Context) {
		calls = append(calls, "unminimise")
	}
	app.alwaysOnTop = func(_ context.Context, enabled bool) {
		if enabled {
			calls = append(calls, "top:true")
			return
		}
		calls = append(calls, "top:false")
	}

	app.restoreWindow()

	want := []string{"show", "unminimise", "top:true", "top:false"}
	if len(calls) != len(want) {
		t.Fatalf("restoreWindow() calls = %v, want %v", calls, want)
	}
	for i := range want {
		if calls[i] != want[i] {
			t.Fatalf("restoreWindow() calls = %v, want %v", calls, want)
		}
	}
}

func TestShutdownStopsManagedDaemonOnlyWhenOwned(t *testing.T) {
	managed := &fakeManagedDaemon{}
	app := NewApp()
	app.managedDaemon = managed
	app.shutdown(context.Background())
	if managed.stopCalls != 1 {
		t.Fatalf("shutdown() managed daemon stop calls = %d, want 1", managed.stopCalls)
	}

	app = NewApp()
	app.shutdown(context.Background())
}

type fakeTray struct {
	showCalls  int
	closeCalls int
	onOpen     func()
	onQuit     func()
}

func (t *fakeTray) Show(onOpen func(), onQuit func()) error {
	t.showCalls++
	t.onOpen = onOpen
	t.onQuit = onQuit
	return nil
}

func (t *fakeTray) Close() {
	t.closeCalls++
}

type fakeManagedDaemon struct {
	stopCalls int
}

func (d *fakeManagedDaemon) Stop(context.Context) error {
	d.stopCalls++
	return nil
}
