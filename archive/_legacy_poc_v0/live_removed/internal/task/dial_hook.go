package task

import (
	"context"
	"io"

	"github.com/miopunch/miopunch/internal/pocstate"
)

// DialPeerStreamHook overrides the peer dialing behavior for tests.
// When set, tasks like `ping`, `sh_ls`, and `sh_attach` will use the returned
// stream instead of performing MQTT signaling and NAT punching.
type DialPeerStreamHook func(ctx context.Context, taskID string, peerID string, cfg pocstate.PeerConfig) (io.ReadWriteCloser, error)

func (m *Manager) SetDialPeerStreamHook(h DialPeerStreamHook) {
	if m == nil {
		return
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	m.dialPeerStreamHook = h
}
