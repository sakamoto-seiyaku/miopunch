package localapi

import (
	"net/http"
	"time"

	"github.com/miopunch/miopunch/internal/task"
)

type Server struct {
	startedAt time.Time
	mode      ListenMode
	tasks     *task.Manager
}

func NewServer(mode ListenMode, tasks *task.Manager) *Server {
	if tasks == nil {
		tasks = task.NewManager()
	}
	return &Server{
		startedAt: time.Now().UTC(),
		mode:      mode,
		tasks:     tasks,
	}
}

func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()

	mux.HandleFunc("GET /api/v0/status", s.handleStatus)
	mux.HandleFunc("GET /api/v0/peers", s.handlePeers)
	mux.HandleFunc("GET /api/v0/tasks", s.handleTasks)
	mux.HandleFunc("GET /api/v0/tasks/{task_id}", s.handleTask)
	mux.HandleFunc("POST /api/v0/tasks", s.handleCreateTask)
	mux.HandleFunc("GET /api/v0/tasks/{task_id}/report", s.handleTaskReport)
	mux.HandleFunc("GET /api/v0/events", s.handleEvents)
	mux.HandleFunc("GET /api/v0/tasks/{task_id}/events", s.handleTaskEvents)
	mux.HandleFunc("GET /api/v0/tasks/{task_id}/ws", s.handleTaskWS)

	return RequireLocalAPIHost(mux)
}
