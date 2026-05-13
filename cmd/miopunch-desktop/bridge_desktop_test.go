//go:build desktop

package main

import (
	"archive/zip"
	"context"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
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

func TestRedactDiagnosticsText(t *testing.T) {
	tests := []struct {
		name  string
		value string
		want  string
	}{
		{
			name:  "json string",
			value: `{"invite_code": "invite-secret"}`,
			want:  `{"invite_code": "[REDACTED]"}`,
		},
		{
			name:  "equals",
			value: `net_secret_b64=net-secret`,
			want:  `net_secret_b64=[REDACTED]`,
		},
		{
			name:  "colon",
			value: `token: token-secret`,
			want:  `token: [REDACTED]`,
		},
		{
			name:  "space",
			value: `password password-secret`,
			want:  `password [REDACTED]`,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := redactDiagnosticsText(tt.value); got != tt.want {
				t.Fatalf("redactDiagnosticsText(%q) = %q, want %q", tt.value, got, tt.want)
			}
		})
	}
}

func TestWriteDiagnosticsArchiveRedactsSnapshotsAndLogs(t *testing.T) {
	logDir := executableLogDir(t)
	desktopLogPath := filepath.Join(logDir, desktopLogFileName)
	daemonLogPath := filepath.Join(logDir, daemonDiagnosticsLogFileName)
	writeTestFile(t, desktopLogPath, "invite_code=desktop-secret\nprivate_key: desktop-private\n")
	writeTestFile(t, daemonLogPath, "net_secret_b64=daemon-secret\npassword daemon-password\n")
	t.Cleanup(func() {
		_ = os.Remove(desktopLogPath)
		_ = os.Remove(daemonLogPath)
	})

	outPath := filepath.Join(t.TempDir(), "diagnostics.zip")
	app := NewApp()
	snapshot := task.DesktopStateSnapshot{
		Rev: 7,
		Tasks: []task.Task{
			{
				ID: "task-redaction-check",
				Facts: []poc.Fact{
					{Message: "invite_code=task-secret"},
					{Message: "token: task-token"},
				},
			},
		},
		Diagnostics: []poc.Fact{
			{Message: "secret_key=diagnostic-secret"},
		},
	}

	if err := app.writeDiagnosticsArchive(outPath, snapshot); err != nil {
		t.Fatalf("writeDiagnosticsArchive(%q) error = %v", outPath, err)
	}

	files := readZipFiles(t, outPath)
	for _, name := range []string{
		"desktop-state.json",
		"connection.json",
		"logs/miopunch-desktop.log",
		"logs/miopunch.log",
	} {
		if _, ok := files[name]; !ok {
			t.Fatalf("diagnostics archive missing %q; entries=%v", name, zipFileNames(files))
		}
	}
	if _, ok := files["data/state.json"]; ok {
		t.Fatalf("diagnostics archive contains raw state file")
	}

	allContent := strings.Join([]string{
		files["desktop-state.json"],
		files["connection.json"],
		files["logs/miopunch-desktop.log"],
		files["logs/miopunch.log"],
	}, "\n")
	for _, secret := range []string{
		"task-secret",
		"task-token",
		"diagnostic-secret",
		"desktop-secret",
		"desktop-private",
		"daemon-secret",
		"daemon-password",
	} {
		if strings.Contains(allContent, secret) {
			t.Fatalf("diagnostics archive contains unredacted secret %q in:\n%s", secret, allContent)
		}
	}
	if !strings.Contains(allContent, "[REDACTED]") {
		t.Fatalf("diagnostics archive content = %q, want redacted markers", allContent)
	}
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

func executableLogDir(t *testing.T) string {
	t.Helper()

	exe, err := os.Executable()
	if err != nil {
		t.Fatalf("os.Executable() error = %v", err)
	}
	logDir := filepath.Join(filepath.Dir(exe), "logs")
	if err := os.MkdirAll(logDir, 0o755); err != nil {
		t.Fatalf("os.MkdirAll(%q) error = %v", logDir, err)
	}
	return logDir
}

func writeTestFile(t *testing.T, path string, content string) {
	t.Helper()

	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatalf("os.WriteFile(%q) error = %v", path, err)
	}
}

func readZipFiles(t *testing.T, path string) map[string]string {
	t.Helper()

	zr, err := zip.OpenReader(path)
	if err != nil {
		t.Fatalf("zip.OpenReader(%q) error = %v", path, err)
	}
	defer func() { _ = zr.Close() }()

	out := make(map[string]string, len(zr.File))
	for _, f := range zr.File {
		rc, err := f.Open()
		if err != nil {
			t.Fatalf("open zip entry %q error = %v", f.Name, err)
		}
		data, err := io.ReadAll(rc)
		_ = rc.Close()
		if err != nil {
			t.Fatalf("read zip entry %q error = %v", f.Name, err)
		}
		out[f.Name] = string(data)
	}
	return out
}

func zipFileNames(files map[string]string) []string {
	names := make([]string, 0, len(files))
	for name := range files {
		names = append(names, name)
	}
	return names
}
