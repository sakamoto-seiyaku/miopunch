package http_panel

import (
	"net"
	"net/http"
	"time"

	"github.com/miopunch/miopunch/internal/task"
)

type Server struct {
	startedAt    time.Time
	listenAddr   string
	origin       string
	allowedHosts map[string]struct{}

	tasks  *task.Manager
	static http.Handler
}

func NewServer(listenAddr string, tasks *task.Manager) *Server {
	if tasks == nil {
		tasks = task.NewManager()
	}

	origin, allowedHosts := panelOrigins(listenAddr)

	return &Server{
		startedAt:    time.Now().UTC(),
		listenAddr:   listenAddr,
		origin:       origin,
		allowedHosts: allowedHosts,
		tasks:        tasks,
		static:       assetsFileServer(),
	}
}

func (s *Server) Origin() string {
	return s.origin
}

func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()

	mux.HandleFunc("GET /", s.handleIndex)
	mux.Handle("GET /assets/", http.StripPrefix("/assets/", s.static))

	mux.HandleFunc("GET /api/v0/status", s.handleStatus)
	mux.HandleFunc("GET /api/v0/peers", s.handlePeers)
	mux.HandleFunc("GET /api/v0/tasks", s.handleTasks)
	mux.HandleFunc("GET /api/v0/tasks/{task_id}", s.handleTask)
	mux.HandleFunc("GET /api/v0/events", s.handleEvents)
	mux.HandleFunc("GET /api/v0/tasks/{task_id}/events", s.handleTaskEvents)
	mux.HandleFunc("GET /api/v0/tasks/{task_id}/report", s.handleTaskReport)

	mux.HandleFunc("POST /api/v0/tasks", s.handleCreateTask)
	mux.HandleFunc("GET /api/v0/tasks/{task_id}/ws", s.handleTaskWS)

	return mux
}

func panelOrigins(listenAddr string) (string, map[string]struct{}) {
	origin := "http://" + listenAddr
	allowedHosts := map[string]struct{}{
		listenAddr: {},
	}

	host, port, err := net.SplitHostPort(listenAddr)
	if err == nil && host == "127.0.0.1" {
		allowedHosts["localhost:"+port] = struct{}{}
	}

	return origin, allowedHosts
}
