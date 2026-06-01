//go:build desktop

package main

import (
	"archive/zip"
	"context"
	"errors"
	"io"
	"net"
	"os"
	"path/filepath"
	goruntime "runtime"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/desktopbridge"
	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
	"github.com/miopunch/miopunch/internal/sessionconfig"
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

func TestStopEventsPumpClosesActiveEventStream(t *testing.T) {
	body := newBlockingRuntimeEventsBody()
	app := NewApp()

	snapshot, err := app.startRuntimeEventsPump(&scriptRuntimeEventsOpener{
		bodies: []io.ReadCloser{body},
	})
	if err != nil {
		t.Fatalf("startRuntimeEventsPump() error = %v, want nil", err)
	}
	if snapshot.Stage != pocruntime.StageNetwork {
		t.Fatalf("startRuntimeEventsPump() snapshot Stage = %q, want %q", snapshot.Stage, pocruntime.StageNetwork)
	}

	app.stopEventsPump()

	if got := body.closeCalls(); got != 1 {
		t.Fatalf("stopEventsPump() stream close calls = %d, want 1", got)
	}
	app.mu.Lock()
	active := app.eventsBody
	done := app.eventsDone
	app.mu.Unlock()
	if active != nil {
		t.Fatalf("stopEventsPump() active stream = %v, want nil", active)
	}
	if done != nil {
		t.Fatalf("stopEventsPump() eventsDone = %v, want nil", done)
	}
}

func TestShutdownStopsEventPumpAndManagedDaemon(t *testing.T) {
	body := newBlockingRuntimeEventsBody()
	managed := &fakeManagedDaemon{}
	app := NewApp()
	app.managedDaemon = managed

	if _, err := app.startRuntimeEventsPump(&scriptRuntimeEventsOpener{
		bodies: []io.ReadCloser{body},
	}); err != nil {
		t.Fatalf("startRuntimeEventsPump() error = %v, want nil", err)
	}

	app.shutdown(context.Background())

	if got := body.closeCalls(); got != 1 {
		t.Fatalf("shutdown() stream close calls = %d, want 1", got)
	}
	if managed.stopCalls != 1 {
		t.Fatalf("shutdown() managed daemon stop calls = %d, want 1", managed.stopCalls)
	}
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

func TestSaveDesktopConfigRequiresLogLevel(t *testing.T) {
	app := NewApp()

	got := app.SaveDesktopConfig(DesktopConfigUpdate{})
	if got.OK {
		t.Fatalf("SaveDesktopConfig(empty).OK = true, want false")
	}
	if got.Error == nil {
		t.Fatalf("SaveDesktopConfig(empty).Error = nil, want error")
	}
	if got.Error.ReasonCode != poc.ReasonCodeBadRequest {
		t.Fatalf("SaveDesktopConfig(empty).Error.ReasonCode = %q, want %q", got.Error.ReasonCode, poc.ReasonCodeBadRequest)
	}
}

func TestSaveDesktopConfigPersistsAndAppliesLogLevel(t *testing.T) {
	if goruntime.GOOS == "windows" {
		t.Skip("unix listener test")
	}

	configPath := filepath.Join(t.TempDir(), "data", "session_config.json")
	oldSessionConfigPath := desktopSessionConfigPath
	desktopSessionConfigPath = func() (string, error) {
		return configPath, nil
	}
	t.Cleanup(func() { desktopSessionConfigPath = oldSessionConfigPath })

	runtimeInstance, err := pocruntime.Open(pocruntime.Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("runtime.Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = runtimeInstance.Close() })

	socketPath := filepath.Join(t.TempDir(), "localapi.sock")
	listener, err := net.Listen("unix", socketPath)
	if err != nil {
		t.Fatalf("net.Listen(unix) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = listener.Close() })

	server := localapi.NewServer(localapi.ListenModeUser, runtimeInstance)
	t.Cleanup(func() { _ = server.Close() })
	go func() { _ = server.Serve(listener) }()

	client, err := localapi.NewClient(localapi.Addr{Transport: localapi.TransportUnix, Path: socketPath})
	if err != nil {
		t.Fatalf("localapi.NewClient() error = %v, want nil", err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for {
		if err := client.ProbeStatus(context.Background()); err == nil {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("ProbeStatus() did not succeed before deadline")
		}
		time.Sleep(20 * time.Millisecond)
	}

	app := NewApp()
	app.client = client
	app.connState = desktopbridge.ConnectionState{
		Connected: true,
		Selected:  desktopbridge.EndpointUser,
		Addr:      "unix:" + socketPath,
	}

	got := app.SaveDesktopConfig(DesktopConfigUpdate{
		Preferences: DesktopConfigPreferences{LogLevel: "debug"},
	})
	if !got.OK {
		t.Fatalf("SaveDesktopConfig(debug).OK = false, error = %+v", got.Error)
	}
	if got.State == nil {
		t.Fatalf("SaveDesktopConfig(debug).State = nil, want snapshot")
	}
	if got.State.Config.Effective.Preferences.LogLevel != "debug" {
		t.Fatalf("SaveDesktopConfig(debug).State.Config.Effective.Preferences.LogLevel = %q, want debug", got.State.Config.Effective.Preferences.LogLevel)
	}
	config, err := sessionconfig.ReadFile(configPath)
	if err != nil {
		t.Fatalf("sessionconfig.ReadFile(%q) error = %v, want nil", configPath, err)
	}
	if config.Preferences.LogLevel != "debug" {
		t.Fatalf("sessionconfig.ReadFile(%q).Preferences.LogLevel = %q, want debug", configPath, config.Preferences.LogLevel)
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

type blockingRuntimeEventsBody struct {
	mu     sync.Mutex
	once   sync.Once
	data   []byte
	closed chan struct{}
	closes int
}

func newBlockingRuntimeEventsBody() *blockingRuntimeEventsBody {
	return &blockingRuntimeEventsBody{
		data:   []byte(`{"kind":"snapshot","snapshot":{"stage":"Network","summary":{"text":"ready"},"evidence":{"facts":[],"suggestions":[]},"discover_view":{},"peer_sessions":[],"shell_sessions":[]}}` + "\n"),
		closed: make(chan struct{}),
	}
}

func (b *blockingRuntimeEventsBody) Read(p []byte) (int, error) {
	b.mu.Lock()
	if len(b.data) > 0 {
		n := copy(p, b.data)
		b.data = b.data[n:]
		b.mu.Unlock()
		return n, nil
	}
	b.mu.Unlock()

	<-b.closed
	return 0, io.ErrClosedPipe
}

func (b *blockingRuntimeEventsBody) Close() error {
	b.once.Do(func() {
		b.mu.Lock()
		b.closes++
		b.mu.Unlock()
		close(b.closed)
	})
	return nil
}

func (b *blockingRuntimeEventsBody) closeCalls() int {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.closes
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
