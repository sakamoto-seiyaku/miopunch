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

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
)

func TestRunRuntimeEventsPumpClosedBeforeSnapshotFailsBootstrap(t *testing.T) {
	opener := &scriptRuntimeEventsOpener{
		bodies: []io.ReadCloser{
			io.NopCloser(strings.NewReader("")),
		},
	}
	firstSnapshotCh := make(chan localapi.Snapshot, 1)
	firstErrCh := make(chan error, 1)

	var app App
	app.runRuntimeEventsPump(context.Background(), nil, opener, firstSnapshotCh, firstErrCh)

	select {
	case got := <-firstErrCh:
		if !errors.Is(got, errDesktopEventStreamClosed) {
			t.Fatalf("runRuntimeEventsPump() bootstrap error = %v, want %v", got, errDesktopEventStreamClosed)
		}
	default:
		t.Fatalf("runRuntimeEventsPump() bootstrap error was not sent")
	}
	select {
	case snapshot := <-firstSnapshotCh:
		t.Fatalf("runRuntimeEventsPump() snapshot = %+v, want none", snapshot)
	default:
	}
}

func TestRunRuntimeEventsPumpClosedAfterSnapshotEmitsRetrying(t *testing.T) {
	opener := &scriptRuntimeEventsOpener{
		bodies: []io.ReadCloser{
			io.NopCloser(strings.NewReader(strings.Join([]string{
				`{"kind":"snapshot","snapshot":{"stage":"Network","summary":{"text":"ready"},"evidence":{"facts":[],"suggestions":[]},"discover_view":{},"peer_sessions":[],"shell_sessions":[]}}`,
				"",
			}, "\n"))),
		},
	}
	firstSnapshotCh := make(chan localapi.Snapshot, 1)
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
		app.runRuntimeEventsPump(ctx, nil, opener, firstSnapshotCh, firstErrCh)
	}()

	snapshot := receiveRuntimeSnapshot(t, firstSnapshotCh)
	if snapshot.Stage != pocruntime.StageNetwork {
		t.Fatalf("runRuntimeEventsPump() snapshot Stage = %q, want %q", snapshot.Stage, pocruntime.StageNetwork)
	}

	ev := receiveDesktopRuntimeEvent(t, runtimeEvents)
	if ev.Kind != "stream_retrying" {
		t.Fatalf("runRuntimeEventsPump() runtime event Kind = %q, want %q", ev.Kind, "stream_retrying")
	}
	if ev.Error == nil || !strings.Contains(ev.Error.Message, errDesktopEventStreamClosed.Error()) {
		t.Fatalf("runRuntimeEventsPump() runtime event Error = %+v, want stream closed error", ev.Error)
	}

	select {
	case got := <-firstErrCh:
		t.Fatalf("runRuntimeEventsPump() bootstrap error = %v, want none", got)
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
	snapshot := localapi.Snapshot{
		Stage:      pocruntime.StageShell,
		ReasonCode: poc.ReasonCodeOK,
		Summary:    pocruntime.UserSummary{Text: "shell gate is satisfied"},
		Evidence: pocruntime.Evidence{
			Facts: []poc.Fact{
				{Message: "invite_code=task-secret"},
				{Message: "token: task-token"},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		},
	}

	if err := app.writeDiagnosticsArchive(outPath, snapshot); err != nil {
		t.Fatalf("writeDiagnosticsArchive(%q) error = %v", outPath, err)
	}

	files := readZipFiles(t, outPath)
	for _, name := range []string{
		"runtime-snapshot.json",
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
		files["runtime-snapshot.json"],
		files["connection.json"],
		files["logs/miopunch-desktop.log"],
		files["logs/miopunch.log"],
	}, "\n")
	for _, secret := range []string{
		"task-secret",
		"task-token",
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

type scriptRuntimeEventsOpener struct {
	bodies []io.ReadCloser
	calls  int
}

func (o *scriptRuntimeEventsOpener) OpenEvents(ctx context.Context) (io.ReadCloser, error) {
	if o.calls < len(o.bodies) {
		body := o.bodies[o.calls]
		o.calls++
		return body, nil
	}
	<-ctx.Done()
	return nil, ctx.Err()
}

func receiveRuntimeSnapshot(t *testing.T, ch <-chan localapi.Snapshot) localapi.Snapshot {
	t.Helper()

	select {
	case snapshot := <-ch:
		return snapshot
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for runtime snapshot")
		return localapi.Snapshot{}
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
	return filepath.Join(filepath.Dir(exe), "logs")
}

func writeTestFile(t *testing.T, path string, content string) {
	t.Helper()

	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("MkdirAll(%q) error = %v", path, err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("WriteFile(%q) error = %v", path, err)
	}
}

func readZipFiles(t *testing.T, path string) map[string]string {
	t.Helper()

	r, err := zip.OpenReader(path)
	if err != nil {
		t.Fatalf("zip.OpenReader(%q) error = %v", path, err)
	}
	defer func() { _ = r.Close() }()

	files := make(map[string]string, len(r.File))
	for _, f := range r.File {
		rc, err := f.Open()
		if err != nil {
			t.Fatalf("File.Open(%q) error = %v", f.Name, err)
		}
		data, err := io.ReadAll(rc)
		_ = rc.Close()
		if err != nil {
			t.Fatalf("ReadAll(%q) error = %v", f.Name, err)
		}
		files[f.Name] = string(data)
	}
	return files
}

func zipFileNames(files map[string]string) []string {
	names := make([]string, 0, len(files))
	for name := range files {
		names = append(names, name)
	}
	return names
}
