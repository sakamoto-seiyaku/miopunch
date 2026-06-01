package controlplane

import (
	"errors"
	"testing"
	"time"
)

func TestInviteIdempotency_RestartReplayDoesNotConsumeUsesAgain(t *testing.T) {
	stateDir := t.TempDir()

	now := time.Unix(10, 0).UTC()
	clock := func() time.Time { return now }

	store1, err := NewInviteStore(stateDir)
	if err != nil {
		t.Fatalf("NewInviteStore() error = %v", err)
	}
	h1, err := NewInviteIdempotency(store1, clock)
	if err != nil {
		t.Fatalf("NewInviteIdempotency() error = %v", err)
	}

	inviteTopic := "invite-topic-1"
	inviteExpiresAtUnixMs := now.Add(1 * time.Hour).UnixMilli()
	maxUses := 1

	req := Message{
		Route: Route{
			MsgID:           testBase32ID("req-1"),
			CreatedAtUnixMs: now.UnixMilli(),
			ExpiresAtUnixMs: now.Add(30 * time.Second).UnixMilli(),
		},
		Signed: Signed{
			Kind: "approve_request",
			Body: []byte(`{"n":1}`),
		},
	}

	calls1 := 0
	resp1, fromCache1, err := h1.Handle(req, inviteTopic, inviteExpiresAtUnixMs, maxUses, func() ([]byte, error) {
		calls1++
		return []byte("ct1"), nil
	})
	if err != nil {
		t.Fatalf("Handle() error = %v", err)
	}
	if fromCache1 {
		t.Fatalf("fromCache1 = %t, want %t", fromCache1, false)
	}
	if calls1 != 1 {
		t.Fatalf("calls1 = %d, want %d", calls1, 1)
	}
	if string(resp1) != "ct1" {
		t.Fatalf("resp1 = %q, want %q", string(resp1), "ct1")
	}

	store2, err := NewInviteStore(stateDir)
	if err != nil {
		t.Fatalf("NewInviteStore(store2) error = %v", err)
	}
	h2, err := NewInviteIdempotency(store2, clock)
	if err != nil {
		t.Fatalf("NewInviteIdempotency(h2) error = %v", err)
	}

	calls2 := 0
	resp2, fromCache2, err := h2.Handle(req, inviteTopic, inviteExpiresAtUnixMs, maxUses, func() ([]byte, error) {
		calls2++
		return []byte("ct2"), nil
	})
	if err != nil {
		t.Fatalf("Handle(replay) error = %v", err)
	}
	if !fromCache2 {
		t.Fatalf("fromCache2 = %t, want %t", fromCache2, true)
	}
	if calls2 != 0 {
		t.Fatalf("calls2 = %d, want %d", calls2, 0)
	}
	if string(resp2) != "ct1" {
		t.Fatalf("resp2 = %q, want %q", string(resp2), "ct1")
	}

	// New request should fail due to uses exhaustion.
	req2 := req
	req2.Route.MsgID = testBase32ID("req-2")
	req2.Route.CreatedAtUnixMs = now.UnixMilli()
	req2.Route.ExpiresAtUnixMs = now.Add(30 * time.Second).UnixMilli()
	_, _, err = h2.Handle(req2, inviteTopic, inviteExpiresAtUnixMs, maxUses, func() ([]byte, error) {
		return []byte("ct3"), nil
	})
	if !errors.Is(err, ErrInviteUsesExhausted) {
		t.Fatalf("Handle(new req) error = %v, want ErrInviteUsesExhausted", err)
	}
}
