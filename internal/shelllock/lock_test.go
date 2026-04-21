package shelllock

import (
	"errors"
	"testing"
	"time"
)

func TestManager_Acquire_TTLExpiry(t *testing.T) {
	t.Parallel()

	now := time.Unix(0, 0).UTC()
	m := NewWithNow(10*time.Second, func() time.Time { return now })
	key := Key{PeerID: "peer1", Target: "local", Session: "main"}

	h1, err := m.Acquire(key)
	if err != nil {
		t.Fatalf("Acquire() error = %v, want nil", err)
	}
	t.Cleanup(h1.Release)

	if _, err := m.Acquire(key); !errors.Is(err, ErrInUse) {
		t.Fatalf("Acquire(locked) error = %v, want %v", err, ErrInUse)
	}

	now = now.Add(11 * time.Second)
	h2, err := m.Acquire(key)
	if err != nil {
		t.Fatalf("Acquire(expired) error = %v, want nil", err)
	}
	h2.Release()
}

func TestHandle_Touch_ExtendsTTL(t *testing.T) {
	t.Parallel()

	now := time.Unix(0, 0).UTC()
	m := NewWithNow(10*time.Second, func() time.Time { return now })
	key := Key{PeerID: "peer1", Target: "local", Session: "main"}

	h, err := m.Acquire(key)
	if err != nil {
		t.Fatalf("Acquire() error = %v, want nil", err)
	}
	t.Cleanup(h.Release)

	now = now.Add(5 * time.Second)
	if ok := h.Touch(); !ok {
		t.Fatalf("Handle.Touch() = false, want true")
	}

	now = now.Add(6 * time.Second) // t=11s, but last activity is t=5s.
	if _, err := m.Acquire(key); !errors.Is(err, ErrInUse) {
		t.Fatalf("Acquire(touched) error = %v, want %v", err, ErrInUse)
	}

	now = now.Add(5 * time.Second) // t=16s; > 10s since last activity at 5s.
	h2, err := m.Acquire(key)
	if err != nil {
		t.Fatalf("Acquire(afterTTL) error = %v, want nil", err)
	}
	h2.Release()
}

func TestHandle_Release(t *testing.T) {
	t.Parallel()

	now := time.Unix(0, 0).UTC()
	m := NewWithNow(10*time.Second, func() time.Time { return now })
	key := Key{PeerID: "peer1", Target: "local", Session: "main"}

	h, err := m.Acquire(key)
	if err != nil {
		t.Fatalf("Acquire() error = %v, want nil", err)
	}

	h.Release()
	h.Release()

	h2, err := m.Acquire(key)
	if err != nil {
		t.Fatalf("Acquire(afterRelease) error = %v, want nil", err)
	}
	h2.Release()
}

func TestHandle_TouchAfterRelease(t *testing.T) {
	t.Parallel()

	now := time.Unix(0, 0).UTC()
	m := NewWithNow(10*time.Second, func() time.Time { return now })
	key := Key{PeerID: "peer1", Target: "local", Session: "main"}

	h, err := m.Acquire(key)
	if err != nil {
		t.Fatalf("Acquire() error = %v, want nil", err)
	}

	h.Release()
	if ok := h.Touch(); ok {
		t.Fatalf("Handle.Touch() = true, want false")
	}
}
