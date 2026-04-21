package task

import (
	"context"
	"encoding/json"
	"errors"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
)

type Manager struct {
	mu sync.Mutex

	ctx    context.Context
	cancel context.CancelFunc

	tasks map[string]*Task

	attachByTask map[string]*attachState

	subsAll    map[int]chan Event
	subsByTask map[string]map[int]chan Event
	nextSubID  int

	wg sync.WaitGroup
}

type attachState struct {
	ch   chan struct{}
	once sync.Once
}

func NewManager() *Manager {
	ctx, cancel := context.WithCancel(context.Background())
	return &Manager{
		ctx:          ctx,
		cancel:       cancel,
		tasks:        make(map[string]*Task),
		attachByTask: make(map[string]*attachState),
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
	m.wg.Wait()
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
		m.attachByTask[t.ID] = &attachState{ch: make(chan struct{})}
	}
	m.mu.Unlock()

	// Emit an initial stage event.
	m.publish(Event{
		Kind:       "stage",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     t.ID,
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

// TriggerShellAttach notifies a running `sh_attach` task that a client connected to its WebSocket.
// It is safe to call multiple times; subsequent calls are no-ops.
func (m *Manager) TriggerShellAttach(taskID string) bool {
	m.mu.Lock()
	state, ok := m.attachByTask[taskID]
	m.mu.Unlock()
	if !ok || state == nil {
		return false
	}

	state.once.Do(func() { close(state.ch) })
	return true
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
		m.setStage(taskID, poc.StageSelfDiscovery, "self discovery (stub)")
		m.addFact(taskID, poc.Fact{Message: "stub: invite is not implemented in POC-05"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry after implementing invite in POC-06/07"})
		m.done(taskID, poc.ReasonCodeNotImplemented, poc.ExitCodeBadRequest)
	case "join":
		m.setStage(taskID, poc.StageCandidateExchange, "candidate exchange (stub)")
		m.addFact(taskID, poc.Fact{Message: "stub: join is not implemented in POC-05"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry after implementing join in POC-06/07"})
		m.done(taskID, poc.ReasonCodeNotImplemented, poc.ExitCodeBadRequest)
	case "approve":
		m.setStage(taskID, poc.StagePeerContact, "peer contact (stub)")
		m.addFact(taskID, poc.Fact{Message: "stub: approve is not implemented in POC-05"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry after implementing approve in POC-06/07"})
		m.done(taskID, poc.ReasonCodeNotImplemented, poc.ExitCodeBadRequest)
	case "ping":
		m.setStage(taskID, poc.StageControlPlaneReady, "control plane ready (stub)")
		m.addFact(taskID, poc.Fact{Message: "stub: ping not implemented; returning success for plumbing"})
		m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
	case "sh_ls":
		m.setStage(taskID, poc.StageSessionAttach, "shell list (stub)")
		m.addFact(taskID, poc.Fact{Message: "stub: sh_ls is not implemented in POC-05"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry after implementing sh_ls in POC-06"})
		m.done(taskID, poc.ReasonCodeNotImplemented, poc.ExitCodeBadRequest)
	case "sh_attach":
		m.setStage(taskID, poc.StageSessionAttach, "waiting for websocket attach (stub)")
		m.addFact(taskID, poc.Fact{Message: "stub: sh_attach runtime is not implemented in POC-05"})

		m.mu.Lock()
		state := m.attachByTask[taskID]
		m.mu.Unlock()
		if state == nil {
			m.addFact(taskID, poc.Fact{Message: "missing sh_attach internal waiter state"})
			m.addSuggestion(taskID, poc.Suggestion{Message: "recreate task and retry"})
			m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
			return
		}

		select {
		case <-ctx.Done():
			m.addFact(taskID, poc.Fact{Message: "task context cancelled"})
			m.addSuggestion(taskID, poc.Suggestion{Message: "task cancelled"})
			m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		case <-state.ch:
			m.addSuggestion(taskID, poc.Suggestion{Message: "sh_attach is not implemented yet; see POC-06 roadmap"})
			m.done(taskID, poc.ReasonCodeNotImplemented, poc.ExitCodeBadRequest)
		case <-time.After(30 * time.Second):
			m.addFact(taskID, poc.Fact{Message: "no websocket attach within 30s"})
			m.addSuggestion(taskID, poc.Suggestion{Message: "retry and attach within 30s"})
			m.done(taskID, poc.ReasonCodeTimeout, poc.ExitCodeTimeout)
		}
	case "revoke_member":
		m.setStage(taskID, poc.StageControlPlaneReady, "revoke member (stub)")
		m.addFact(taskID, poc.Fact{Message: "stub: revoke_member is not implemented in POC-05"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry after implementing revoke_member in POC-06/07"})
		m.done(taskID, poc.ReasonCodeNotImplemented, poc.ExitCodeBadRequest)
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

	m.mu.Lock()
	t, ok := m.tasks[taskID]
	if ok {
		t.Stage = stage
		t.Timeline = append(t.Timeline, TimelineEntry{
			At:      time.Now().UTC(),
			Stage:   stage,
			Message: message,
		})
	}
	m.mu.Unlock()

	m.publish(Event{
		Kind:       "stage",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     taskID,
		Stage:      stage,
		Message:    message,
	})
}

func (m *Manager) addFact(taskID string, fact poc.Fact) {
	m.mu.Lock()
	if t, ok := m.tasks[taskID]; ok {
		t.Facts = append(t.Facts, fact)
	}
	m.mu.Unlock()

	m.publish(Event{
		Kind:       "fact",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     taskID,
		Fact:       &fact,
	})
}

func (m *Manager) addSuggestion(taskID string, suggestion poc.Suggestion) {
	m.mu.Lock()
	if t, ok := m.tasks[taskID]; ok {
		t.Suggestions = append(t.Suggestions, suggestion)
	}
	m.mu.Unlock()

	m.publish(Event{
		Kind:       "diagnosis",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     taskID,
		Suggestion: &suggestion,
	})
}

func (m *Manager) done(taskID string, reasonCode poc.ReasonCode, exitCode poc.ExitCode) {
	m.mu.Lock()
	t, ok := m.tasks[taskID]
	if ok {
		t.Status = StatusDone
		t.ReasonCode = reasonCode
		t.ExitCode = exitCode
		t.Report = buildReportMarkdown(t.Clone())
		t.ReportReady = true
	}
	delete(m.attachByTask, taskID)
	m.mu.Unlock()

	m.publish(Event{
		Kind:       "report_ready",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     taskID,
	})
	m.publish(Event{
		Kind:       "done",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     taskID,
		ReasonCode: reasonCode,
		ExitCode:   exitCode,
	})
}
