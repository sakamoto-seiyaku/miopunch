//go:build desktop

package main

import (
	"archive/zip"
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	"regexp"
	"strings"
	"time"

	"github.com/wailsapp/wails/v2/pkg/runtime"

	"github.com/miopunch/miopunch/internal/bundlepath"
	"github.com/miopunch/miopunch/internal/desktopbridge"
	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/sessionconfig"
)

type RuntimeActionResult struct {
	OK     bool                       `json:"ok"`
	Error  *desktopbridge.BridgeError `json:"error,omitempty"`
	Result *localapi.ActionResult     `json:"result,omitempty"`
}

type ExportDiagnosticsResult struct {
	OK        bool                       `json:"ok"`
	Cancelled bool                       `json:"cancelled,omitempty"`
	Error     *desktopbridge.BridgeError `json:"error,omitempty"`
	Path      string                     `json:"path,omitempty"`
}

type DesktopRuntimeResult struct {
	OK         bool                           `json:"ok"`
	Error      *desktopbridge.BridgeError     `json:"error,omitempty"`
	Connection *desktopbridge.ConnectionState `json:"connection,omitempty"`
	State      *localapi.Snapshot             `json:"state,omitempty"`
}

// SaveDesktopConfigResult is returned after saving desktop runtime config.
type SaveDesktopConfigResult struct {
	OK         bool                           `json:"ok"`
	Error      *desktopbridge.BridgeError     `json:"error,omitempty"`
	Connection *desktopbridge.ConnectionState `json:"connection,omitempty"`
	State      *localapi.Snapshot             `json:"state,omitempty"`
}

// DesktopConfigUpdate contains desktop settings changes from the GUI.
type DesktopConfigUpdate struct {
	Preferences DesktopConfigPreferences `json:"preferences,omitempty"`
}

// DesktopConfigPreferences contains user preference updates.
type DesktopConfigPreferences struct {
	LogLevel string `json:"log_level,omitempty"`
}

type DesktopRuntimeEvent struct {
	Kind       string                         `json:"kind"`
	Connection *desktopbridge.ConnectionState `json:"connection,omitempty"`
	Error      *desktopbridge.BridgeError     `json:"error,omitempty"`
}

var errDesktopEventStreamClosed = errors.New("runtime event stream closed")

const (
	daemonDiagnosticsLogFileName = "miopunch.log"
	eventPumpStopTimeout         = 2 * time.Second
)

var diagnosticsRedactor = regexp.MustCompile(`(?i)(secret_key|invite_code|join_code|net_secret_b64|invite_secret_b64|ed25519_seed_b64|x25519_priv_b64|private_key|password|token)(["=: ]+)([^"\s,}]+)`)

type runtimeEventsOpener interface {
	OpenEvents(context.Context) (io.ReadCloser, error)
}

type runtimeEventStream struct {
	body io.Closer
}

func (a *App) Connect() desktopbridge.ConnectionState {
	startedAt := time.Now()
	logutil.Debugf("desktop connect start")
	_, state := a.connectLocalAPI()
	a.emitConnectionState(state)
	logutil.Debugf(
		"desktop connect done: elapsed_ms=%d connected=%t selected=%s addr=%s",
		time.Since(startedAt).Milliseconds(),
		state.Connected,
		state.Selected,
		state.Addr,
	)
	return state
}

func (a *App) DesktopRuntimeStart() DesktopRuntimeResult {
	startedAt := time.Now()
	logutil.Debugf("desktop runtime start begin")
	client, state := a.connectLocalAPI()
	a.emitConnectionState(state)
	if !state.Connected {
		logutil.Debugf(
			"desktop runtime start done: elapsed_ms=%d connected=false reason_code=%s",
			time.Since(startedAt).Milliseconds(),
			bridgeReasonCode(state.Failure),
		)
		return DesktopRuntimeResult{
			OK:         false,
			Error:      state.Failure,
			Connection: &state,
		}
	}

	snapshot, err := a.startRuntimeEventsPump(client)
	if err != nil {
		logutil.Debugf(
			"desktop runtime start done: elapsed_ms=%d connected=true reason_code=%s",
			time.Since(startedAt).Milliseconds(),
			err.ReasonCode,
		)
		return DesktopRuntimeResult{
			OK:         false,
			Error:      err,
			Connection: &state,
		}
	}

	logutil.Debugf(
		"desktop runtime start done: elapsed_ms=%d connected=true stage=%s",
		time.Since(startedAt).Milliseconds(),
		snapshotStage(snapshot),
	)
	return DesktopRuntimeResult{
		OK:         true,
		Connection: &state,
		State:      snapshot,
	}
}

func (a *App) DesktopRuntimeResync() DesktopRuntimeResult {
	startedAt := time.Now()
	logutil.Debugf("desktop runtime resync start")
	c, err := a.localAPIClient()
	if err != nil {
		state := a.ConnectionState()
		logutil.Debugf(
			"desktop runtime resync done: elapsed_ms=%d reason_code=%s",
			time.Since(startedAt).Milliseconds(),
			err.ReasonCode,
		)
		return DesktopRuntimeResult{
			OK:         false,
			Error:      err,
			Connection: &state,
		}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	snapshot, apiErr := c.GetSnapshot(ctx)
	if apiErr != nil {
		state := a.ConnectionState()
		bridgeErr := bridgeErrorFromErr(apiErr)
		logutil.Debugf(
			"desktop runtime resync done: elapsed_ms=%d reason_code=%s",
			time.Since(startedAt).Milliseconds(),
			bridgeReasonCode(bridgeErr),
		)
		return DesktopRuntimeResult{
			OK:         false,
			Error:      bridgeErr,
			Connection: &state,
		}
	}

	state := a.ConnectionState()
	logutil.Debugf(
		"desktop runtime resync done: elapsed_ms=%d stage=%s",
		time.Since(startedAt).Milliseconds(),
		snapshot.Stage,
	)
	return DesktopRuntimeResult{
		OK:         true,
		Connection: &state,
		State:      &snapshot,
	}
}

func (a *App) RuntimeAction(action string, args any) RuntimeActionResult {
	action = strings.TrimSpace(action)
	startedAt := time.Now()
	logutil.Debugf("desktop runtime action start: action=%s", action)
	c, err := a.localAPIClient()
	if err != nil {
		logutil.Debugf(
			"desktop runtime action done: action=%s elapsed_ms=%d reason_code=%s",
			action,
			time.Since(startedAt).Milliseconds(),
			err.ReasonCode,
		)
		return RuntimeActionResult{OK: false, Error: err}
	}

	ctx, cancel := context.WithTimeout(context.Background(), actionTimeout(action))
	defer cancel()

	result, apiErr := c.Action(ctx, action, args)
	if apiErr != nil {
		bridgeErr := bridgeErrorFromErr(apiErr)
		logutil.Debugf(
			"desktop runtime action done: action=%s elapsed_ms=%d reason_code=%s",
			action,
			time.Since(startedAt).Milliseconds(),
			bridgeReasonCode(bridgeErr),
		)
		return RuntimeActionResult{OK: false, Error: bridgeErr}
	}
	logutil.Debugf(
		"desktop runtime action done: action=%s elapsed_ms=%d stage=%s reason_code=%s exit_code=%d",
		action,
		time.Since(startedAt).Milliseconds(),
		result.Stage,
		result.ReasonCode,
		result.ExitCode,
	)
	return RuntimeActionResult{OK: true, Result: &result}
}

// SaveDesktopConfig persists supported desktop settings and applies them now.
func (a *App) SaveDesktopConfig(update DesktopConfigUpdate) SaveDesktopConfigResult {
	startedAt := time.Now()
	rawLevel := strings.TrimSpace(update.Preferences.LogLevel)
	if rawLevel == "" {
		return SaveDesktopConfigResult{OK: false, Error: &desktopbridge.BridgeError{
			Stage:      "desktop",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Message:    "missing log level",
			Suggestions: []poc.Suggestion{
				{Message: "choose a log level before saving"},
			},
		}}
	}
	level, err := sessionconfig.NormalizeLogLevel(rawLevel)
	if err != nil {
		return SaveDesktopConfigResult{OK: false, Error: &desktopbridge.BridgeError{
			Stage:      "desktop",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Message:    "invalid log level",
			Facts:      []poc.Fact{{Message: "log_level=" + rawLevel}},
			Suggestions: []poc.Suggestion{
				{Message: "use trace, debug, info, warn, or error"},
			},
		}}
	}

	path, err := desktopSessionConfigPath()
	if err != nil {
		return SaveDesktopConfigResult{OK: false, Error: bridgeErrorFromErr(err)}
	}
	config, err := sessionconfig.ReadFile(path)
	if err != nil {
		return SaveDesktopConfigResult{OK: false, Error: bridgeErrorFromErr(err)}
	}
	config.Preferences.LogLevel = level
	if err := sessionconfig.WriteFile(path, config); err != nil {
		return SaveDesktopConfigResult{OK: false, Error: bridgeErrorFromErr(err)}
	}
	logutil.SetLevel(level)
	logutil.Infof("desktop log level changed: log_level=%s", level)

	c, bridgeErr := a.localAPIClient()
	if bridgeErr != nil {
		state := a.ConnectionState()
		return SaveDesktopConfigResult{OK: false, Error: bridgeErr, Connection: &state}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	snapshot, apiErr := c.SetLogLevel(ctx, level)
	if apiErr != nil {
		state := a.ConnectionState()
		errResp := bridgeErrorFromErr(apiErr)
		logutil.Debugf(
			"desktop config save done: elapsed_ms=%d reason_code=%s",
			time.Since(startedAt).Milliseconds(),
			bridgeReasonCode(errResp),
		)
		return SaveDesktopConfigResult{OK: false, Error: errResp, Connection: &state}
	}

	state := a.ConnectionState()
	logutil.Debugf(
		"desktop config save done: elapsed_ms=%d log_level=%s",
		time.Since(startedAt).Milliseconds(),
		level,
	)
	return SaveDesktopConfigResult{
		OK:         true,
		Connection: &state,
		State:      &snapshot,
	}
}

func actionTimeout(action string) time.Duration {
	switch action {
	case "approve", "join":
		return 3 * time.Minute
	case "sh":
		return 2 * time.Minute
	default:
		return 30 * time.Second
	}
}

func (a *App) connectLocalAPI() (*localapi.Client, desktopbridge.ConnectionState) {
	a.mu.Lock()
	ctx := a.ctx
	override := a.overrideAddr
	a.mu.Unlock()

	client, selectedAddr, state, managedDaemon := desktopbridge.Connect(ctx, override)

	a.mu.Lock()
	a.client = client
	a.selectedAddr = selectedAddr
	if managedDaemon != nil {
		a.managedDaemon = managedDaemon
	}
	a.connState = state
	a.mu.Unlock()

	if !state.Connected {
		a.stopEventsPump()
	}

	return client, state
}

func (a *App) ConnectionState() desktopbridge.ConnectionState {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.connState
}

func (a *App) SetLocalAPIOverride(addr string) desktopbridge.ConnectionState {
	a.mu.Lock()
	a.overrideAddr = addr
	a.mu.Unlock()
	return a.Connect()
}

func (a *App) ClearLocalAPIOverride() desktopbridge.ConnectionState {
	return a.SetLocalAPIOverride("")
}

func (a *App) ExportDiagnostics() ExportDiagnosticsResult {
	c, err := a.localAPIClient()
	if err != nil {
		return ExportDiagnosticsResult{OK: false, Error: err}
	}

	a.mu.Lock()
	wailsCtx := a.ctx
	a.mu.Unlock()

	path, dialogErr := runtime.SaveFileDialog(wailsCtx, runtime.SaveDialogOptions{
		DefaultFilename: "miopunch-diagnostics.zip",
		Title:           "Export diagnostics",
		Filters: []runtime.FileFilter{
			{DisplayName: "Zip archive", Pattern: "*.zip"},
		},
	})
	if dialogErr != nil {
		return ExportDiagnosticsResult{OK: false, Error: bridgeErrorFromErr(dialogErr)}
	}
	if path == "" {
		return ExportDiagnosticsResult{OK: true, Cancelled: true}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	snapshot, apiErr := c.GetSnapshot(ctx)
	if apiErr != nil {
		return ExportDiagnosticsResult{OK: false, Error: bridgeErrorFromErr(apiErr)}
	}
	if err := a.writeDiagnosticsArchive(path, snapshot); err != nil {
		return ExportDiagnosticsResult{OK: false, Error: bridgeErrorFromErr(err)}
	}
	return ExportDiagnosticsResult{OK: true, Path: path}
}

func (a *App) writeDiagnosticsArchive(path string, snapshot localapi.Snapshot) error {
	out, err := os.Create(path)
	if err != nil {
		return err
	}
	defer func() { _ = out.Close() }()

	zw := zip.NewWriter(out)
	if err := writeZipJSON(zw, "runtime-snapshot.json", snapshot); err != nil {
		_ = zw.Close()
		return err
	}
	if err := writeZipJSON(zw, "connection.json", a.ConnectionState()); err != nil {
		_ = zw.Close()
		return err
	}
	if err := writeDiagnosticsLog(zw, "logs/miopunch-desktop.log", desktopLogFileName); err != nil {
		_ = zw.Close()
		return err
	}
	if err := writeDiagnosticsLog(zw, "logs/miopunch.log", daemonDiagnosticsLogFileName); err != nil {
		_ = zw.Close()
		return err
	}
	return zw.Close()
}

func writeZipJSON(zw *zip.Writer, name string, value any) error {
	w, err := zw.Create(name)
	if err != nil {
		return err
	}
	data, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	_, err = w.Write([]byte(redactDiagnosticsText(string(data))))
	return err
}

func writeDiagnosticsLog(zw *zip.Writer, archiveName string, fileName string) error {
	path, err := bundlepath.LogPath(fileName)
	if err != nil {
		return nil
	}
	data, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil
		}
		return err
	}
	w, err := zw.Create(archiveName)
	if err != nil {
		return err
	}
	_, err = w.Write([]byte(redactDiagnosticsText(string(data))))
	return err
}

func redactDiagnosticsText(value string) string {
	return diagnosticsRedactor.ReplaceAllString(value, `$1$2[REDACTED]`)
}

func (a *App) localAPIClient() (*localapi.Client, *desktopbridge.BridgeError) {
	a.mu.Lock()
	defer a.mu.Unlock()

	if a.client != nil && a.connState.Connected {
		return a.client, nil
	}
	if a.connState.Failure != nil {
		return nil, a.connState.Failure
	}
	return nil, &desktopbridge.BridgeError{
		Stage:      "desktop",
		ReasonCode: poc.ReasonCodeDaemonNotRunning,
		ExitCode:   poc.ExitCodeUnavailable,
		Message:    "LocalAPI is not connected",
		Suggestions: []poc.Suggestion{
			{Message: "retry connection"},
		},
	}
}

func (a *App) stopManagedDaemon() {
	a.mu.Lock()
	managedDaemon := a.managedDaemon
	a.managedDaemon = nil
	a.mu.Unlock()

	if managedDaemon == nil {
		return
	}

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	_ = managedDaemon.Stop(ctx)
}

func bridgeErrorFromErr(err error) *desktopbridge.BridgeError {
	if err == nil {
		return nil
	}

	var apiErr *localapi.APIError
	if errors.As(err, &apiErr) {
		return &desktopbridge.BridgeError{
			Stage:       apiErr.Response.Stage,
			ReasonCode:  apiErr.Response.ReasonCode,
			ExitCode:    apiErr.Response.ExitCode,
			Message:     apiErr.Response.Message,
			Facts:       apiErr.Response.Facts,
			Suggestions: apiErr.Response.Suggestions,
		}
	}

	return &desktopbridge.BridgeError{
		Stage:      "desktop",
		ReasonCode: poc.ReasonCodeInternal,
		ExitCode:   poc.ExitCodeInternal,
		Message:    err.Error(),
		Suggestions: []poc.Suggestion{
			{Message: "retry"},
		},
	}
}

func bridgeReasonCode(err *desktopbridge.BridgeError) poc.ReasonCode {
	if err == nil {
		return poc.ReasonCodeOK
	}
	return err.ReasonCode
}

func snapshotStage(snapshot *localapi.Snapshot) string {
	if snapshot == nil {
		return ""
	}
	return string(snapshot.Stage)
}

func (a *App) emitConnectionState(state desktopbridge.ConnectionState) {
	a.mu.Lock()
	wailsCtx := a.ctx
	a.mu.Unlock()

	if wailsCtx != nil {
		runtime.EventsEmit(wailsCtx, "localapi:connection", state)
		runtime.EventsEmit(wailsCtx, "desktop:runtime", DesktopRuntimeEvent{
			Kind:       "connection",
			Connection: &state,
		})
	}
}

func (a *App) emitRuntimeEvent(ev DesktopRuntimeEvent) {
	a.mu.Lock()
	wailsCtx := a.ctx
	hook := a.runtimeEventHook
	a.mu.Unlock()

	if hook != nil {
		hook(ev)
	}
	if wailsCtx != nil {
		runtime.EventsEmit(wailsCtx, "desktop:runtime", ev)
	}
}

func (a *App) startRuntimeEventsPump(c runtimeEventsOpener) (*localapi.Snapshot, *desktopbridge.BridgeError) {
	if c == nil {
		return nil, bridgeErrorFromErr(errors.New("missing LocalAPI client"))
	}

	a.stopEventsPump()

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	firstSnapshotCh := make(chan localapi.Snapshot, 1)
	firstErrCh := make(chan error, 1)

	a.mu.Lock()
	a.eventsCancel = cancel
	a.eventsDone = done
	wailsCtx := a.ctx
	a.mu.Unlock()

	go func() {
		defer close(done)
		a.runRuntimeEventsPump(ctx, wailsCtx, c, firstSnapshotCh, firstErrCh)
	}()

	select {
	case snapshot := <-firstSnapshotCh:
		return &snapshot, nil
	case err := <-firstErrCh:
		a.stopEventsPump()
		return nil, bridgeErrorFromErr(err)
	case <-time.After(5 * time.Second):
		a.stopEventsPump()
		return nil, bridgeErrorFromErr(context.DeadlineExceeded)
	}
}

func (a *App) stopEventsPump() {
	a.mu.Lock()
	cancel := a.eventsCancel
	done := a.eventsDone
	stream := a.eventsBody
	a.eventsCancel = nil
	a.eventsDone = nil
	a.eventsBody = nil
	a.mu.Unlock()

	if cancel != nil {
		cancel()
	}
	if stream != nil && stream.body != nil {
		_ = stream.body.Close()
	}
	if done != nil {
		select {
		case <-done:
		case <-time.After(eventPumpStopTimeout):
			logutil.Warnf("desktop runtime event pump did not stop before timeout")
		}
	}
}

func (a *App) registerEventStream(body io.Closer) *runtimeEventStream {
	if body == nil {
		return nil
	}
	stream := &runtimeEventStream{body: body}

	a.mu.Lock()
	a.eventsBody = stream
	a.mu.Unlock()
	return stream
}

func (a *App) clearEventStream(stream *runtimeEventStream) {
	if stream == nil {
		return
	}

	a.mu.Lock()
	if a.eventsBody == stream {
		a.eventsBody = nil
	}
	a.mu.Unlock()
}

func (a *App) runRuntimeEventsPump(
	ctx context.Context,
	wailsCtx context.Context,
	c runtimeEventsOpener,
	firstSnapshotCh chan<- localapi.Snapshot,
	firstErrCh chan<- error,
) {
	const retryDelay = 750 * time.Millisecond
	bootstrapPending := true

	for {
		if err := ctx.Err(); err != nil {
			return
		}

		body, err := c.OpenEvents(ctx)
		if err != nil {
			if bootstrapPending {
				select {
				case firstErrCh <- err:
				default:
				}
				return
			}
			a.emitRuntimeEvent(DesktopRuntimeEvent{
				Kind:  "stream_retrying",
				Error: bridgeErrorFromErr(err),
			})
			select {
			case <-time.After(retryDelay):
				continue
			case <-ctx.Done():
				return
			}
		}
		if err := ctx.Err(); err != nil {
			_ = body.Close()
			return
		}
		stream := a.registerEventStream(body)
		if stream == nil {
			_ = body.Close()
			return
		}

		initialStreamSnapshot := true
		readErr := desktopbridge.ReadLocalAPITaskEvents(ctx, body, func(ev localapi.Event) error {
			if initialStreamSnapshot {
				initialStreamSnapshot = false
				if ev.Kind != "snapshot" {
					return errors.New("runtime event stream did not begin with snapshot")
				}
				if bootstrapPending {
					bootstrapPending = false
					select {
					case firstSnapshotCh <- ev.Snapshot:
					default:
					}
					return nil
				}
			}
			if wailsCtx != nil {
				runtime.EventsEmit(wailsCtx, "desktop:state", ev)
			}
			return nil
		})
		a.clearEventStream(stream)
		_ = body.Close()
		if err := ctx.Err(); err != nil {
			return
		}

		streamErr := readErr
		if streamErr == nil {
			streamErr = errDesktopEventStreamClosed
		}
		if bootstrapPending {
			select {
			case firstErrCh <- streamErr:
			default:
			}
			return
		}
		a.emitRuntimeEvent(DesktopRuntimeEvent{
			Kind:  "stream_retrying",
			Error: bridgeErrorFromErr(streamErr),
		})

		select {
		case <-time.After(retryDelay):
			continue
		case <-ctx.Done():
			return
		}
	}
}
