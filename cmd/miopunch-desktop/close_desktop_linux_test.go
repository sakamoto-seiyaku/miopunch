//go:build desktop && linux

package main

import (
	"context"
	"testing"
	"time"
)

func TestBeforeCloseLinuxExitsProcessAndPreventsDefaultClose(t *testing.T) {
	managed := &fakeManagedDaemon{}
	app := NewApp()
	app.managedDaemon = managed

	exitCh := make(chan int, 1)
	app.exitProcess = func(code int) {
		exitCh <- code
	}

	prevent := app.beforeCloseLinux(context.Background())
	if !prevent {
		t.Fatal("beforeCloseLinux() prevent = false, want true")
	}
	if !app.isQuitRequested() {
		t.Fatal("beforeCloseLinux() quitRequested = false, want true")
	}
	select {
	case exitCode := <-exitCh:
		if exitCode != 0 {
			t.Fatalf("beforeCloseLinux() exit code = %d, want 0", exitCode)
		}
	case <-time.After(2 * time.Second):
		t.Fatalf("beforeCloseLinux() did not call exitProcess")
	}
	if managed.stopCalls != 1 {
		t.Fatalf("beforeCloseLinux() managed daemon stop calls = %d, want 1", managed.stopCalls)
	}
}

func TestExitNowRunsCleanupOnlyOnce(t *testing.T) {
	managed := &fakeManagedDaemon{}
	app := NewApp()
	app.managedDaemon = managed

	exitCalls := 0
	app.exitProcess = func(int) {
		exitCalls++
	}

	app.exitNow(0)
	app.exitNow(0)

	if exitCalls != 1 {
		t.Fatalf("exitNow() exit calls = %d, want 1", exitCalls)
	}
	if managed.stopCalls != 1 {
		t.Fatalf("exitNow() managed daemon stop calls = %d, want 1", managed.stopCalls)
	}
}
