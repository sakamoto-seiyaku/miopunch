package task

import (
	"context"
	"encoding/json"
	"errors"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"

	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

type Manager struct {
	mu sync.Mutex

	ctx    context.Context
	cancel context.CancelFunc

	tasks map[string]*Task

	attachByTask map[string]*attachState

	sessions *dataplane.SessionManager

	dialPeerStreamHook DialPeerStreamHook

	topologyMu       sync.Mutex
	topologyAttempts []TopologyAttempt
	topologyPayloads []TopologyPayload

	stateMu   sync.Mutex
	statePath string

	subsAll    map[int]chan Event
	subsByTask map[string]map[int]chan Event
	nextSubID  int

	wg sync.WaitGroup
}

type attachState struct {
	wsCh chan *websocket.Conn
	once sync.Once
}

func NewManager() *Manager {
	statePath, _ := pocstate.DefaultStatePath()
	return NewManagerWithStatePath(statePath)
}

func NewManagerWithStatePath(statePath string) *Manager {
	ctx, cancel := context.WithCancel(context.Background())
	return &Manager{
		ctx:          ctx,
		cancel:       cancel,
		tasks:        make(map[string]*Task),
		attachByTask: make(map[string]*attachState),
		sessions:     dataplane.NewSessionManager(),
		statePath:    strings.TrimSpace(statePath),
		subsAll:      make(map[int]chan Event),
		subsByTask:   make(map[string]map[int]chan Event),
	}
}

func (m *Manager) Wait() {
	m.wg.Wait()
}

func (m *Manager) Close() {
	if m == nil {
		return
	}
	if m.cancel != nil {
		m.cancel()
	}
	if m.sessions != nil {
		m.sessions.CloseAll(dataplane.CloseReasonDaemonShutdown)
	}
	m.wg.Wait()
}

func (m *Manager) loadState() (pocstate.State, error) {
	m.stateMu.Lock()
	defer m.stateMu.Unlock()

	if strings.TrimSpace(m.statePath) == "" {
		return pocstate.State{}, errors.New("missing state path")
	}
	return pocstate.Load(m.statePath)
}

func (m *Manager) saveState(st pocstate.State) error {
	m.stateMu.Lock()
	defer m.stateMu.Unlock()

	if strings.TrimSpace(m.statePath) == "" {
		return errors.New("missing state path")
	}
	return pocstate.Save(m.statePath, st)
}

func (m *Manager) ListPeers() ([]string, error) {
	st, err := m.loadState()
	if err != nil {
		return nil, err
	}

	peerIDs := make([]string, 0, len(st.Peers))
	for peerID := range st.Peers {
		peerIDs = append(peerIDs, peerID)
	}
	sort.Strings(peerIDs)
	return peerIDs, nil
}

func (m *Manager) List() []Task {
	m.mu.Lock()
	defer m.mu.Unlock()

	out := make([]Task, 0, len(m.tasks))
	for _, t := range m.tasks {
		out = append(out, t.Clone())
	}
	sort.Slice(out, func(i, j int) bool {
		return out[i].CreatedAt.Before(out[j].CreatedAt)
	})
	return out
}

func (m *Manager) Get(taskID string) (Task, bool) {
	m.mu.Lock()
	defer m.mu.Unlock()

	t, ok := m.tasks[taskID]
	if !ok {
		return Task{}, false
	}
	return t.Clone(), true
}

func (m *Manager) GetReport(taskID string) (string, bool) {
	m.mu.Lock()
	defer m.mu.Unlock()

	t, ok := m.tasks[taskID]
	if !ok {
		return "", false
	}
	if !t.ReportReady {
		return "", false
	}
	return t.Report, true
}

type Subscription struct {
	C <-chan Event

	closeOnce sync.Once
	closeFn   func()
}

func (s *Subscription) Close() {
	if s == nil {
		return
	}
	s.closeOnce.Do(func() {
		if s.closeFn != nil {
			s.closeFn()
		}
	})
}

func (m *Manager) SubscribeAll() *Subscription {
	m.mu.Lock()
	defer m.mu.Unlock()

	id := m.nextSubID
	m.nextSubID++
	ch := make(chan Event, 1)
	m.subsAll[id] = ch

	return &Subscription{
		C: ch,
		closeFn: func() {
			m.mu.Lock()
			defer m.mu.Unlock()
			if c, ok := m.subsAll[id]; ok {
				delete(m.subsAll, id)
				close(c)
			}
		},
	}
}

func (m *Manager) SubscribeTask(taskID string) *Subscription {
	m.mu.Lock()
	defer m.mu.Unlock()

	id := m.nextSubID
	m.nextSubID++
	ch := make(chan Event, 1)
	if _, ok := m.subsByTask[taskID]; !ok {
		m.subsByTask[taskID] = make(map[int]chan Event)
	}
	m.subsByTask[taskID][id] = ch

	return &Subscription{
		C: ch,
		closeFn: func() {
			m.mu.Lock()
			defer m.mu.Unlock()
			subs, ok := m.subsByTask[taskID]
			if !ok {
				return
			}
			if c, ok := subs[id]; ok {
				delete(subs, id)
				close(c)
			}
			if len(subs) == 0 {
				delete(m.subsByTask, taskID)
			}
		},
	}
}

func (m *Manager) publish(ev Event) {
	m.mu.Lock()
	defer m.mu.Unlock()

	for _, ch := range m.subsAll {
		sendLatest(ch, ev)
	}

	if ev.TaskID == "" {
		return
	}
	for _, ch := range m.subsByTask[ev.TaskID] {
		sendLatest(ch, ev)
	}
}

func sendLatest(ch chan Event, ev Event) {
	select {
	case ch <- ev:
		return
	default:
	}

	// Best-effort: drop the oldest buffered event (if any) and retry so
	// subscribers eventually observe the latest state transition (e.g. `done`).
	select {
	case <-ch:
	default:
	}
	select {
	case ch <- ev:
	default:
	}
}

type CreateRequest struct {
	Kind string          `json:"kind"`
	Args json.RawMessage `json:"args,omitempty"`
}

func (m *Manager) CreateAndRun(req CreateRequest) (Task, error) {
	if strings.TrimSpace(req.Kind) == "" {
		return Task{}, errors.New("empty task kind")
	}

	taskID, err := poc.NewTaskID()
	if err != nil {
		return Task{}, err
	}

	t := &Task{
		ID:          taskID,
		Kind:        req.Kind,
		CreatedAt:   time.Now().UTC(),
		Status:      StatusRunning,
		Stage:       poc.StageControlPlaneReady,
		Facts:       []poc.Fact{},
		Suggestions: []poc.Suggestion{},
	}

	m.mu.Lock()
	m.tasks[t.ID] = t
	if req.Kind == "sh_attach" {
		m.attachByTask[t.ID] = &attachState{wsCh: make(chan *websocket.Conn, 1)}
	}
	snapshot := t.Clone()
	m.mu.Unlock()

	// Emit an initial stage event.
	m.publish(Event{
		Kind:       "stage",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     t.ID,
		Task:       &snapshot,
		Stage:      t.Stage,
		Message:    "started",
	})

	m.wg.Add(1)
	go func() {
		defer m.wg.Done()
		m.runStub(m.ctx, t.ID, req)
	}()

	return t.Clone(), nil
}

// AttachShellWS hands a LocalAPI WebSocket connection to a running `sh_attach` task.
// It is safe to call multiple times; subsequent calls are no-ops and return false.
func (m *Manager) AttachShellWS(taskID string, conn *websocket.Conn) bool {
	if conn == nil {
		return false
	}

	m.mu.Lock()
	state, ok := m.attachByTask[taskID]
	m.mu.Unlock()
	if !ok || state == nil {
		return false
	}

	added := false
	state.once.Do(func() {
		state.wsCh <- conn
		added = true
	})
	return added
}

func (m *Manager) runStub(ctx context.Context, taskID string, req CreateRequest) {
	select {
	case <-ctx.Done():
		m.addFact(taskID, poc.Fact{Message: "task context cancelled"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "task cancelled"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	default:
	}

	switch req.Kind {
	case "invite":
		m.runInviteTask(taskID, req.Args)
	case "join":
		m.runJoinTask(taskID, req.Args)
	case "approve":
		m.runApproveTask(taskID, req.Args)
	case "ping":
		m.runPingTask(taskID, req.Args)
	case "bootstrap_more":
		m.runBootstrapMoreTask(taskID, req.Args)
	case "maintain_neighbors":
		m.runMaintainNeighborsTask(taskID, req.Args)
	case "sh_ls":
		m.runShellListTask(taskID, req.Args)
	case "sh_attach":
		m.runShellAttachTask(taskID, req.Args)
	case "revoke_member":
		m.runRevokeMemberTask(taskID, req.Args)
	default:
		m.setStage(taskID, poc.StageControlPlaneReady, "not implemented (stub)")
		m.addFact(taskID, poc.Fact{Message: "stub: unsupported task kind"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "this task is not implemented yet; see POC-06/07 roadmap"})
		m.done(taskID, poc.ReasonCodeNotImplemented, poc.ExitCodeBadRequest)
	}
}

func (m *Manager) setStage(taskID string, stage poc.Stage, message string) {
	if !poc.IsValidStage(stage) {
		stage = poc.StageControlPlaneReady
	}

	kind := ""
	var snapshot *Task
	m.mu.Lock()
	t, ok := m.tasks[taskID]
	if ok {
		kind = t.Kind
		t.Stage = stage
		t.Timeline = append(t.Timeline, TimelineEntry{
			At:      time.Now().UTC(),
			Stage:   stage,
			Message: message,
		})
		snapshot = cloneTaskForEvent(t)
	}
	m.mu.Unlock()

	if ok {
		logTaskStage(taskID, kind, stage, message)
	}

	m.publish(Event{
		Kind:       "stage",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     taskID,
		Task:       snapshot,
		Stage:      stage,
		Message:    message,
	})
}

func (m *Manager) addFact(taskID string, fact poc.Fact) {
	kind := ""
	ok := false
	var snapshot *Task
	m.mu.Lock()
	if t, found := m.tasks[taskID]; found {
		kind = t.Kind
		ok = true
		t.Facts = append(t.Facts, fact)
		snapshot = cloneTaskForEvent(t)
	}
	m.mu.Unlock()

	if ok {
		logTaskFact(taskID, kind, fact)
	}

	m.publish(Event{
		Kind:       "fact",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     taskID,
		Task:       snapshot,
		Fact:       &fact,
	})
}

func (m *Manager) addSuggestion(taskID string, suggestion poc.Suggestion) {
	kind := ""
	ok := false
	var snapshot *Task
	m.mu.Lock()
	if t, found := m.tasks[taskID]; found {
		kind = t.Kind
		ok = true
		t.Suggestions = append(t.Suggestions, suggestion)
		snapshot = cloneTaskForEvent(t)
	}
	m.mu.Unlock()

	if ok {
		logTaskSuggestion(taskID, kind, suggestion)
	}

	m.publish(Event{
		Kind:       "diagnosis",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     taskID,
		Task:       snapshot,
		Suggestion: &suggestion,
	})
}

func (m *Manager) done(taskID string, reasonCode poc.ReasonCode, exitCode poc.ExitCode) {
	var report string
	var reportReady bool
	kind := ""
	var snapshot *Task

	m.mu.Lock()
	t, ok := m.tasks[taskID]
	if ok {
		kind = t.Kind
		t.Status = StatusDone
		t.ReasonCode = reasonCode
		t.ExitCode = exitCode
		report = buildReportMarkdown(t.Clone())
		t.Report = report
		t.ReportReady = true
		reportReady = true
		snapshot = cloneTaskForEvent(t)
	}
	delete(m.attachByTask, taskID)
	m.mu.Unlock()

	if reportReady {
		if err := m.persistReport(taskID, report); err != nil {
			m.addFact(taskID, poc.Fact{Message: "persist task report failed: " + err.Error()})
			m.addSuggestion(taskID, poc.Suggestion{Message: "fix state_dir permissions/disk space; then retry"})

			m.mu.Lock()
			if t, ok := m.tasks[taskID]; ok {
				t.Report = buildReportMarkdown(t.Clone())
				snapshot = cloneTaskForEvent(t)
			}
			m.mu.Unlock()
		}
	}

	if ok {
		logTaskDone(taskID, kind, reasonCode, exitCode)
	}

	m.publish(Event{
		Kind:       "report_ready",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     taskID,
		Task:       snapshot,
	})
	m.publish(Event{
		Kind:       "done",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     taskID,
		Task:       snapshot,
		ReasonCode: reasonCode,
		ExitCode:   exitCode,
	})
}

func cloneTaskForEvent(t *Task) *Task {
	if t == nil {
		return nil
	}
	snapshot := t.Clone()
	return &snapshot
}
