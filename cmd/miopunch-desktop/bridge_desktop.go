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
	"time"

	"github.com/wailsapp/wails/v2/pkg/runtime"

	"github.com/miopunch/miopunch/internal/bundlepath"
	"github.com/miopunch/miopunch/internal/desktopbridge"
	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/task"
)

type StatusResult struct {
	OK     bool                       `json:"ok"`
	Error  *desktopbridge.BridgeError `json:"error,omitempty"`
	Status *localapi.StatusResponse   `json:"status,omitempty"`
}

type PeersResult struct {
	OK    bool                       `json:"ok"`
	Error *desktopbridge.BridgeError `json:"error,omitempty"`
	Peers *localapi.PeersResponse    `json:"peers,omitempty"`
}

type TasksResult struct {
	OK    bool                       `json:"ok"`
	Error *desktopbridge.BridgeError `json:"error,omitempty"`
	Tasks *localapi.TasksResponse    `json:"tasks,omitempty"`
}

type TopologyResult struct {
	OK       bool                       `json:"ok"`
	Error    *desktopbridge.BridgeError `json:"error,omitempty"`
	Topology *task.TopologySnapshot     `json:"topology,omitempty"`
}

type TaskResult struct {
	OK    bool                       `json:"ok"`
	Error *desktopbridge.BridgeError `json:"error,omitempty"`
	Task  *task.Task                 `json:"task,omitempty"`
}

type ReportResult struct {
	OK     bool                       `json:"ok"`
	Error  *desktopbridge.BridgeError `json:"error,omitempty"`
	Report string                     `json:"report,omitempty"`
}

type CreateTaskResult struct {
	OK    bool                       `json:"ok"`
	Error *desktopbridge.BridgeError `json:"error,omitempty"`
	Task  *task.Task                 `json:"task,omitempty"`
}

type ExportReportResult struct {
	OK        bool                       `json:"ok"`
	Cancelled bool                       `json:"cancelled,omitempty"`
	Error     *desktopbridge.BridgeError `json:"error,omitempty"`
	Path      string                     `json:"path,omitempty"`
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
	State      *task.DesktopStateSnapshot     `json:"state,omitempty"`
}

type DesktopRuntimeEvent struct {
	Kind       string                         `json:"kind"`
	Connection *desktopbridge.ConnectionState `json:"connection,omitempty"`
	Error      *desktopbridge.BridgeError     `json:"error,omitempty"`
}

var errDesktopEventStreamClosed = errors.New("desktop event stream closed")

const daemonDiagnosticsLogFileName = "miopunch.log"

var diagnosticsRedactor = regexp.MustCompile(`(?i)(secret_key|invite_code|join_code|net_secret_b64|invite_secret_b64|ed25519_seed_b64|x25519_priv_b64|private_key|password|token)(["=: ]+)([^"\s,}]+)`)

type desktopEventsOpener interface {
	OpenDesktopEvents(context.Context) (io.ReadCloser, error)
}

func (a *App) Connect() desktopbridge.ConnectionState {
	_, state := a.connectLocalAPI()
	a.emitConnectionState(state)
	return state
}

func (a *App) DesktopRuntimeStart() DesktopRuntimeResult {
	client, state := a.connectLocalAPI()
	a.emitConnectionState(state)
	if !state.Connected {
		return DesktopRuntimeResult{
			OK:         false,
			Error:      state.Failure,
			Connection: &state,
		}
	}

	snapshot, err := a.startDesktopEventsPump(client)
	if err != nil {
		return DesktopRuntimeResult{
			OK:         false,
			Error:      err,
			Connection: &state,
		}
	}

	return DesktopRuntimeResult{
		OK:         true,
		Connection: &state,
		State:      snapshot,
	}
}

func (a *App) DesktopRuntimeResync() DesktopRuntimeResult {
	c, err := a.localAPIClient()
	if err != nil {
		state := a.ConnectionState()
		return DesktopRuntimeResult{
			OK:         false,
			Error:      err,
			Connection: &state,
		}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	snapshot, apiErr := c.GetDesktopState(ctx)
	if apiErr != nil {
		state := a.ConnectionState()
		return DesktopRuntimeResult{
			OK:         false,
			Error:      bridgeErrorFromErr(apiErr),
			Connection: &state,
		}
	}

	state := a.ConnectionState()
	return DesktopRuntimeResult{
		OK:         true,
		Connection: &state,
		State:      &snapshot,
	}
}

func (a *App) SaveDesktopConfig(update task.DesktopConfigUpdate) DesktopRuntimeResult {
	c, err := a.localAPIClient()
	if err != nil {
		state := a.ConnectionState()
		return DesktopRuntimeResult{
			OK:         false,
			Error:      err,
			Connection: &state,
		}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	snapshot, apiErr := c.UpdateDesktopConfig(ctx, update)
	if apiErr != nil {
		state := a.ConnectionState()
		return DesktopRuntimeResult{
			OK:         false,
			Error:      bridgeErrorFromErr(apiErr),
			Connection: &state,
		}
	}

	state := a.ConnectionState()
	return DesktopRuntimeResult{
		OK:         true,
		Connection: &state,
		State:      &snapshot,
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

func (a *App) GetStatus() StatusResult {
	c, err := a.localAPIClient()
	if err != nil {
		return StatusResult{OK: false, Error: err}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	status, apiErr := c.GetStatus(ctx)
	if apiErr != nil {
		return StatusResult{OK: false, Error: bridgeErrorFromErr(apiErr)}
	}

	return StatusResult{OK: true, Status: &status}
}

func (a *App) GetPeers() PeersResult {
	c, err := a.localAPIClient()
	if err != nil {
		return PeersResult{OK: false, Error: err}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	peers, apiErr := c.GetPeers(ctx)
	if apiErr != nil {
		return PeersResult{OK: false, Error: bridgeErrorFromErr(apiErr)}
	}

	return PeersResult{OK: true, Peers: &peers}
}

func (a *App) GetTasks() TasksResult {
	c, err := a.localAPIClient()
	if err != nil {
		return TasksResult{OK: false, Error: err}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	tasksResp, apiErr := c.GetTasks(ctx)
	if apiErr != nil {
		return TasksResult{OK: false, Error: bridgeErrorFromErr(apiErr)}
	}

	return TasksResult{OK: true, Tasks: &tasksResp}
}

func (a *App) GetTopology() TopologyResult {
	c, err := a.localAPIClient()
	if err != nil {
		return TopologyResult{OK: false, Error: err}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	topology, apiErr := c.GetTopology(ctx)
	if apiErr != nil {
		return TopologyResult{OK: false, Error: bridgeErrorFromErr(apiErr)}
	}

	return TopologyResult{OK: true, Topology: &topology}
}

func (a *App) GetTask(taskID string) TaskResult {
	c, err := a.localAPIClient()
	if err != nil {
		return TaskResult{OK: false, Error: err}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	t, apiErr := c.GetTask(ctx, taskID)
	if apiErr != nil {
		return TaskResult{OK: false, Error: bridgeErrorFromErr(apiErr)}
	}

	return TaskResult{OK: true, Task: &t}
}

func (a *App) GetTaskReport(taskID string) ReportResult {
	c, err := a.localAPIClient()
	if err != nil {
		return ReportResult{OK: false, Error: err}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	report, apiErr := c.GetTaskReport(ctx, taskID)
	if apiErr != nil {
		return ReportResult{OK: false, Error: bridgeErrorFromErr(apiErr)}
	}

	return ReportResult{OK: true, Report: report}
}

func (a *App) CreateTask(kind string, args any) CreateTaskResult {
	c, err := a.localAPIClient()
	if err != nil {
		return CreateTaskResult{OK: false, Error: err}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	t, apiErr := c.CreateTask(ctx, kind, args)
	if apiErr != nil {
		return CreateTaskResult{OK: false, Error: bridgeErrorFromErr(apiErr)}
	}

	return CreateTaskResult{OK: true, Task: &t}
}

func (a *App) ExportTaskReport(taskID string) ExportReportResult {
	c, err := a.localAPIClient()
	if err != nil {
		return ExportReportResult{OK: false, Error: err}
	}

	a.mu.Lock()
	wailsCtx := a.ctx
	a.mu.Unlock()

	path, dialogErr := runtime.SaveFileDialog(wailsCtx, runtime.SaveDialogOptions{
		DefaultFilename: taskID + ".md",
		Title:           "Export report",
		Filters: []runtime.FileFilter{
			{DisplayName: "Markdown", Pattern: "*.md"},
		},
	})
	if dialogErr != nil {
		return ExportReportResult{OK: false, Error: bridgeErrorFromErr(dialogErr)}
	}
	if path == "" {
		return ExportReportResult{OK: true, Cancelled: true}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	report, apiErr := c.GetTaskReport(ctx, taskID)
	if apiErr != nil {
		return ExportReportResult{OK: false, Error: bridgeErrorFromErr(apiErr)}
	}

	if err := os.WriteFile(path, []byte(report), 0o644); err != nil {
		return ExportReportResult{OK: false, Error: bridgeErrorFromErr(err)}
	}

	return ExportReportResult{OK: true, Path: path}
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

	snapshot, apiErr := c.GetDesktopState(ctx)
	if apiErr != nil {
		return ExportDiagnosticsResult{OK: false, Error: bridgeErrorFromErr(apiErr)}
	}
	if err := a.writeDiagnosticsArchive(path, snapshot); err != nil {
		return ExportDiagnosticsResult{OK: false, Error: bridgeErrorFromErr(err)}
	}
	return ExportDiagnosticsResult{OK: true, Path: path}
}

func (a *App) writeDiagnosticsArchive(path string, snapshot task.DesktopStateSnapshot) error {
	out, err := os.Create(path)
	if err != nil {
		return err
	}
	defer func() { _ = out.Close() }()

	zw := zip.NewWriter(out)
	if err := writeZipJSON(zw, "desktop-state.json", snapshot); err != nil {
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

func (a *App) startDesktopEventsPump(c *localapi.Client) (*task.DesktopStateSnapshot, *desktopbridge.BridgeError) {
	if c == nil {
		return nil, bridgeErrorFromErr(errors.New("missing LocalAPI client"))
	}

	a.stopEventsPump()

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	firstSnapshotCh := make(chan task.DesktopStateSnapshot, 1)
	firstErrCh := make(chan error, 1)

	a.mu.Lock()
	a.eventsCancel = cancel
	a.eventsDone = done
	wailsCtx := a.ctx
	a.mu.Unlock()

	go func() {
		defer close(done)
		a.runDesktopEventsPump(ctx, wailsCtx, c, firstSnapshotCh, firstErrCh)
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
	a.eventsCancel = nil
	a.eventsDone = nil
	a.mu.Unlock()

	if cancel != nil {
		cancel()
	}
	if done != nil {
		<-done
	}
}

func (a *App) runDesktopEventsPump(
	ctx context.Context,
	wailsCtx context.Context,
	c desktopEventsOpener,
	firstSnapshotCh chan<- task.DesktopStateSnapshot,
	firstErrCh chan<- error,
) {
	const retryDelay = 750 * time.Millisecond
	bootstrapPending := true

	for {
		if err := ctx.Err(); err != nil {
			return
		}

		body, err := c.OpenDesktopEvents(ctx)
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

		initialStreamSnapshot := true
		readErr := desktopbridge.ReadLocalAPIDesktopStateEvents(ctx, body, func(ev task.DesktopStateEvent) error {
			if initialStreamSnapshot {
				initialStreamSnapshot = false
				if ev.Kind != task.DesktopStateEventSnapshot || ev.Snapshot == nil {
					return errors.New("desktop event stream did not begin with snapshot")
				}
				if bootstrapPending {
					bootstrapPending = false
					select {
					case firstSnapshotCh <- *ev.Snapshot:
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
