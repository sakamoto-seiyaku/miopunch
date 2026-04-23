package localapi

import (
	"time"

	"github.com/miopunch/miopunch/internal/task"
)

// StatusResponse is the JSON response body for GET /api/v0/status.
type StatusResponse struct {
	Version   string     `json:"version"`
	StartedAt time.Time  `json:"started_at"`
	UptimeMs  int64      `json:"uptime_ms"`
	Mode      ListenMode `json:"mode"`
}

// PeersResponse is the JSON response body for GET /api/v0/peers.
type PeersResponse struct {
	Peers []Peer `json:"peers"`
}

// Peer is a minimal peer descriptor returned by GET /api/v0/peers.
type Peer struct {
	PeerID string `json:"peer_id"`
}

// TasksResponse is the JSON response body for GET /api/v0/tasks.
type TasksResponse struct {
	Tasks []task.Task `json:"tasks"`
}
