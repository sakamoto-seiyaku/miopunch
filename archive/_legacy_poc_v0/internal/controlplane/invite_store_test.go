package controlplane

import (
	"errors"
	"testing"
	"time"
)

func TestInviteIDFromTopic_Deterministic(t *testing.T) {
	id1, err := InviteIDFromTopic("abc")
	if err != nil {
		t.Fatalf("InviteIDFromTopic() error = %v", err)
	}
	id2, err := InviteIDFromTopic("abc")
	if err != nil {
		t.Fatalf("InviteIDFromTopic() second error = %v", err)
	}
	if id1 != id2 {
		t.Fatalf("invite_id mismatch: %q vs %q", id1, id2)
	}
	if len(id1) != 26 {
		t.Fatalf("invite_id length = %d, want %d", len(id1), 26)
	}
}

func TestInviteStore_HandleRequest_DoesNotDoubleDecrement(t *testing.T) {
	stateDir := t.TempDir()

	store, err := NewInviteStore(stateDir)
	if err != nil {
		t.Fatalf("NewInviteStore() error = %v", err)
	}

	inviteTopic := "invite-topic-1"
	inviteExpiresAtUnixMs := time.Now().UTC().Add(1 * time.Hour).UnixMilli()
	maxUses := 1

	reqID := testBase32ID("req-1")

	calls := 0
	resp1, fromCache1, err := store.HandleRequest(inviteTopic, inviteExpiresAtUnixMs, maxUses, reqID, func() ([]byte, error) {
		calls++
		return []byte("ct1"), nil
	})
	if err != nil {
		t.Fatalf("HandleRequest() error = %v", err)
	}
	if fromCache1 {
		t.Fatalf("fromCache1 = %t, want %t", fromCache1, false)
	}
	if calls != 1 {
		t.Fatalf("calls = %d, want %d", calls, 1)
	}
	if string(resp1) != "ct1" {
		t.Fatalf("resp1 = %q, want %q", string(resp1), "ct1")
	}

	resp2, fromCache2, err := store.HandleRequest(inviteTopic, inviteExpiresAtUnixMs, maxUses, reqID, func() ([]byte, error) {
		t.Fatalf("unexpected build on handled hit")
		return nil, nil
	})
	if err != nil {
		t.Fatalf("HandleRequest(hit) error = %v", err)
	}
	if !fromCache2 {
		t.Fatalf("fromCache2 = %t, want %t", fromCache2, true)
	}
	if string(resp2) != "ct1" {
		t.Fatalf("resp2 = %q, want %q", string(resp2), "ct1")
	}

	inviteID, err := store.EnsureInvite(inviteTopic, inviteExpiresAtUnixMs, maxUses)
	if err != nil {
		t.Fatalf("EnsureInvite() error = %v", err)
	}
	rec, err := store.loadInvite(inviteID)
	if err != nil {
		t.Fatalf("loadInvite() error = %v", err)
	}
	if rec.UsesLeft != 0 {
		t.Fatalf("uses_left = %d, want %d", rec.UsesLeft, 0)
	}

	_, _, err = store.HandleRequest(inviteTopic, inviteExpiresAtUnixMs, maxUses, testBase32ID("req-2"), func() ([]byte, error) {
		return []byte("ct2"), nil
	})
	if !errors.Is(err, ErrInviteUsesExhausted) {
		t.Fatalf("HandleRequest(uses exhausted) error = %v, want ErrInviteUsesExhausted", err)
	}
}

func TestInviteStore_RestartRecovery_ReturnsCachedResponse(t *testing.T) {
	stateDir := t.TempDir()

	store1, err := NewInviteStore(stateDir)
	if err != nil {
		t.Fatalf("NewInviteStore() error = %v", err)
	}

	inviteTopic := "invite-topic-1"
	inviteExpiresAtUnixMs := time.Now().UTC().Add(1 * time.Hour).UnixMilli()
	maxUses := 1
	reqID := testBase32ID("req-1")

	_, _, err = store1.HandleRequest(inviteTopic, inviteExpiresAtUnixMs, maxUses, reqID, func() ([]byte, error) {
		return []byte("ct1"), nil
	})
	if err != nil {
		t.Fatalf("HandleRequest(store1) error = %v", err)
	}

	store2, err := NewInviteStore(stateDir)
	if err != nil {
		t.Fatalf("NewInviteStore(store2) error = %v", err)
	}

	calls := 0
	resp2, fromCache2, err := store2.HandleRequest(inviteTopic, inviteExpiresAtUnixMs, maxUses, reqID, func() ([]byte, error) {
		calls++
		return []byte("ct2"), nil
	})
	if err != nil {
		t.Fatalf("HandleRequest(store2) error = %v", err)
	}
	if !fromCache2 {
		t.Fatalf("fromCache2 = %t, want %t", fromCache2, true)
	}
	if calls != 0 {
		t.Fatalf("calls = %d, want %d", calls, 0)
	}
	if string(resp2) != "ct1" {
		t.Fatalf("resp2 = %q, want %q", string(resp2), "ct1")
	}
}
