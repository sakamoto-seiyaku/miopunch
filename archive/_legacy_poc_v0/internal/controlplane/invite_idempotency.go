package controlplane

import (
	"errors"
	"time"
)

var (
	ErrInviteExpired = errors.New("invite expired")
	ErrNotRPCRequest = errors.New("not rpc request")
)

// InviteIdempotency wires RPC time semantics with issuer-side invite accounting
// (uses + handled request caching).
type InviteIdempotency struct {
	store *InviteStore
	clock func() time.Time
}

func NewInviteIdempotency(store *InviteStore, clock func() time.Time) (*InviteIdempotency, error) {
	if store == nil {
		return nil, errors.New("nil invite store")
	}
	if clock == nil {
		clock = func() time.Time { return time.Now().UTC() }
	}
	return &InviteIdempotency{
		store: store,
		clock: clock,
	}, nil
}

// Handle applies invite/approve idempotency for a single RPC request.
//
// It validates receiver-side RPC time semantics and invite expiry, then delegates
// uses accounting + handled response caching to InviteStore.
func (h *InviteIdempotency) Handle(req Message, inviteTopic string, inviteExpiresAtUnixMs int64, maxUses int, buildFinalResponseCiphertext func() ([]byte, error)) ([]byte, bool, error) {
	if h == nil || h.store == nil {
		return nil, false, errors.New("nil invite idempotency")
	}
	if !IsRPCRequest(req.Signed.Kind) {
		return nil, false, ErrNotRPCRequest
	}

	nowUnixMs := h.clock().UTC().UnixMilli()
	if err := ValidateRPCRequestTime(nowUnixMs, req); err != nil {
		return nil, false, err
	}
	if inviteExpiresAtUnixMs > 0 && nowUnixMs > inviteExpiresAtUnixMs {
		return nil, false, ErrInviteExpired
	}

	return h.store.HandleRequest(inviteTopic, inviteExpiresAtUnixMs, maxUses, req.Route.MsgID, buildFinalResponseCiphertext)
}
