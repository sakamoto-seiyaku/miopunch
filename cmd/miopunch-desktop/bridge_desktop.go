//go:build desktop

package main

import (
	"context"
	"errors"
	"os"
	"time"

	"github.com/wailsapp/wails/v2/pkg/runtime"

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

func (a *App) Connect() desktopbridge.ConnectionState {
	a.mu.Lock()
	ctx := a.ctx
	override := a.overrideAddr
	a.mu.Unlock()

	client, selectedAddr, state := desktopbridge.Connect(ctx, override)

	a.mu.Lock()
	a.client = client
	a.selectedAddr = selectedAddr
	a.connState = state
	a.mu.Unlock()

	if state.Connected {
		a.startEventsPump(client)
	} else {
		a.stopEventsPump()
	}

	a.emitConnectionState(state)
	return state
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
	}
}

func (a *App) startEventsPump(c *localapi.Client) {
	if c == nil {
		return
	}

	a.stopEventsPump()

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})

	a.mu.Lock()
	a.eventsCancel = cancel
	a.eventsDone = done
	wailsCtx := a.ctx
	a.mu.Unlock()

	go func() {
		defer close(done)
		a.runEventsPump(ctx, wailsCtx, c)
	}()
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

func (a *App) runEventsPump(ctx context.Context, wailsCtx context.Context, c *localapi.Client) {
	const retryDelay = 750 * time.Millisecond

	for {
		if err := ctx.Err(); err != nil {
			return
		}

		body, err := c.OpenEvents(ctx)
		if err != nil {
			select {
			case <-time.After(retryDelay):
				continue
			case <-ctx.Done():
				return
			}
		}

		_ = desktopbridge.ReadLocalAPITaskEvents(ctx, body, func(ev task.Event) error {
			if wailsCtx != nil {
				runtime.EventsEmit(wailsCtx, "localapi:event", ev)
			}
			return nil
		})
		_ = body.Close()

		select {
		case <-time.After(retryDelay):
			continue
		case <-ctx.Done():
			return
		}
	}
}
