package dataplane

import "context"

// PeerSessionListener accepts inbound peer transport sessions.
//
// Implementations MUST support multiple sequential Accept calls and MUST honor
// context cancellation for deterministic shutdown.
type PeerSessionListener interface {
	Accept(ctx context.Context) (PeerSession, error)
	Close() error
}
