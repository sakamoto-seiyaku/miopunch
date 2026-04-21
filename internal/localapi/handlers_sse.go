package localapi

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/task"
)

const sseHeartbeatInterval = 15 * time.Second

func (s *Server) handleEvents(w http.ResponseWriter, r *http.Request) {
	flusher, ok := w.(http.Flusher)
	if !ok {
		reqID, _ := poc.NewRequestID()
		writeError(w, ErrorResponse{
			Stage:      "localapi",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Message:    "streaming not supported",
			Facts:      []poc.Fact{},
			Suggestions: []poc.Suggestion{
				{Message: "retry with a client that supports HTTP streaming"},
			},
			RequestID: reqID,
		})
		return
	}

	sub := s.tasks.SubscribeAll()
	defer sub.Close()

	startSSE(w)
	flusher.Flush()

	_ = writeSSEEvent(w, task.Event{
		Kind:       "snapshot",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		Tasks:      s.tasks.List(),
	})
	flusher.Flush()

	serveSSELoop(r, w, flusher, sub.C)
}

func (s *Server) handleTaskEvents(w http.ResponseWriter, r *http.Request) {
	rawTaskID := strings.TrimSpace(r.PathValue("task_id"))
	taskID, err := poc.CanonicalizeTaskID(rawTaskID)
	if err != nil {
		writeBadRequest(w, fmt.Sprintf("invalid task_id: %v", err))
		return
	}

	t, ok := s.tasks.Get(taskID)
	if !ok {
		writeNotFound(w, taskID, "task not found")
		return
	}

	flusher, ok := w.(http.Flusher)
	if !ok {
		reqID, _ := poc.NewRequestID()
		writeError(w, ErrorResponse{
			Stage:      "localapi",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Message:    "streaming not supported",
			Facts:      []poc.Fact{},
			Suggestions: []poc.Suggestion{
				{Message: "retry with a client that supports HTTP streaming"},
			},
			RequestID: reqID,
		})
		return
	}

	sub := s.tasks.SubscribeTask(taskID)
	defer sub.Close()

	startSSE(w)
	flusher.Flush()

	_ = writeSSEEvent(w, task.Event{
		Kind:       "snapshot",
		TimeUnixMs: time.Now().UTC().UnixMilli(),
		TaskID:     taskID,
		Task:       &t,
	})
	flusher.Flush()

	serveSSELoop(r, w, flusher, sub.C)
}

func startSSE(w http.ResponseWriter) {
	h := w.Header()
	h.Set("Content-Type", "text/event-stream; charset=utf-8")
	h.Set("Cache-Control", "no-cache")
	h.Set("Connection", "keep-alive")
	h.Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(http.StatusOK)
}

func serveSSELoop(r *http.Request, w http.ResponseWriter, flusher http.Flusher, ch <-chan task.Event) {
	ticker := time.NewTicker(sseHeartbeatInterval)
	defer ticker.Stop()

	for {
		select {
		case <-r.Context().Done():
			return
		case <-ticker.C:
			_, _ = fmt.Fprint(w, ": ping\n\n")
			flusher.Flush()
		case ev, ok := <-ch:
			if !ok {
				return
			}
			_ = writeSSEEvent(w, ev)
			flusher.Flush()
		}
	}
}

func writeSSEEvent(w http.ResponseWriter, ev task.Event) error {
	b, err := json.Marshal(ev)
	if err != nil {
		return err
	}
	_, err = fmt.Fprintf(w, "data: %s\n\n", b)
	return err
}
