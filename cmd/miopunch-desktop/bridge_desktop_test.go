//go:build desktop

package main

import (
	"context"
	"errors"
	"io"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/task"
)

func TestRunDesktopEventsPump_ClosedBeforeSnapshotFailsBootstrap(t *testing.T) {
	opener := &scriptDesktopEventsOpener{
		bodies: []io.ReadCloser{
			io.NopCloser(strings.NewReader("")),
		},
	}
	firstSnapshotCh := make(chan task.DesktopStateSnapshot, 1)
	firstErrCh := make(chan error, 1)

	var app App
	app.runDesktopEventsPump(context.Background(), nil, opener, firstSnapshotCh, firstErrCh)

	select {
	case got := <-firstErrCh:
		if !errors.Is(got, errDesktopEventStreamClosed) {
			t.Fatalf("runDesktopEventsPump() bootstrap error = %v, want %v", got, errDesktopEventStreamClosed)
		}
	default:
		t.Fatalf("runDesktopEventsPump() bootstrap error was not sent")
	}
	select {
	case snapshot := <-firstSnapshotCh:
		t.Fatalf("runDesktopEventsPump() snapshot = %+v, want none", snapshot)
	default:
	}
}

func TestRunDesktopEventsPump_ClosedAfterSnapshotEmitsRetrying(t *testing.T) {
	opener := &scriptDesktopEventsOpener{
		bodies: []io.ReadCloser{
			io.NopCloser(strings.NewReader(strings.Join([]string{
				`data: {"kind":"snapshot","snapshot":{"rev":1}}`,
				"",
			}, "\n"))),
		},
	}
	firstSnapshotCh := make(chan task.DesktopStateSnapshot, 1)
	firstErrCh := make(chan error, 1)
	runtimeEvents := make(chan DesktopRuntimeEvent, 1)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	app := &App{
		runtimeEventHook: func(ev DesktopRuntimeEvent) {
			runtimeEvents <- ev
			cancel()
		},
	}
	done := make(chan struct{})
	go func() {
		defer close(done)
		app.runDesktopEventsPump(ctx, nil, opener, firstSnapshotCh, firstErrCh)
	}()

	snapshot := receiveDesktopSnapshot(t, firstSnapshotCh)
	if snapshot.Rev != 1 {
		t.Fatalf("runDesktopEventsPump() snapshot Rev = %d, want 1", snapshot.Rev)
	}

	ev := receiveDesktopRuntimeEvent(t, runtimeEvents)
	if ev.Kind != "stream_retrying" {
		t.Fatalf("runDesktopEventsPump() runtime event Kind = %q, want %q", ev.Kind, "stream_retrying")
	}
	if ev.Error == nil || !strings.Contains(ev.Error.Message, errDesktopEventStreamClosed.Error()) {
		t.Fatalf("runDesktopEventsPump() runtime event Error = %+v, want stream closed error", ev.Error)
	}

	select {
	case got := <-firstErrCh:
		t.Fatalf("runDesktopEventsPump() bootstrap error = %v, want none", got)
	default:
	}
	waitDesktopPumpDone(t, done)
}

type scriptDesktopEventsOpener struct {
	bodies []io.ReadCloser
	calls  int
}

func (o *scriptDesktopEventsOpener) OpenDesktopEvents(ctx context.Context) (io.ReadCloser, error) {
	if o.calls < len(o.bodies) {
		body := o.bodies[o.calls]
		o.calls++
		return body, nil
	}
	<-ctx.Done()
	return nil, ctx.Err()
}

func receiveDesktopSnapshot(t *testing.T, ch <-chan task.DesktopStateSnapshot) task.DesktopStateSnapshot {
	t.Helper()

	select {
	case snapshot := <-ch:
		return snapshot
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for desktop snapshot")
		return task.DesktopStateSnapshot{}
	}
}

func receiveDesktopRuntimeEvent(t *testing.T, ch <-chan DesktopRuntimeEvent) DesktopRuntimeEvent {
	t.Helper()

	select {
	case ev := <-ch:
		return ev
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for desktop runtime event")
		return DesktopRuntimeEvent{}
	}
}

func waitDesktopPumpDone(t *testing.T, done <-chan struct{}) {
	t.Helper()

	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for desktop event pump to stop")
	}
}
