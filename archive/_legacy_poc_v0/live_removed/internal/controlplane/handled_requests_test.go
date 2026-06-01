package controlplane

import (
	"errors"
	"sync"
	"testing"
	"time"
)

func TestHandledRequestsCache_Hit(t *testing.T) {
	now := time.Unix(0, 0).UTC()
	clock := func() time.Time { return now }

	c := NewHandledRequestsCache(2, 10*time.Second, clock)

	req := Message{
		Route: Route{
			DstPeerID:       testBase32ID("dst"),
			MsgID:           testBase32ID("req-1"),
			CreatedAtUnixMs: 1,
			ExpiresAtUnixMs: now.Add(30 * time.Second).UnixMilli(),
		},
		Signed: Signed{
			SenderPeerID: testBase32ID("sender"),
			Kind:         "echo_request",
			Body:         []byte(`{"n":1}`),
		},
	}

	calls := 0
	build := func() ([]byte, error) {
		calls++
		return []byte("resp"), nil
	}

	resp1, fromCache1, err := c.Handle(req, build)
	if err != nil {
		t.Fatalf("Handle() error = %v", err)
	}
	if fromCache1 {
		t.Fatalf("Handle() fromCache = %t, want %t", fromCache1, false)
	}
	if calls != 1 {
		t.Fatalf("calls = %d, want %d", calls, 1)
	}
	if string(resp1) != "resp" {
		t.Fatalf("resp1 = %q, want %q", string(resp1), "resp")
	}

	resp2, fromCache2, err := c.Handle(req, func() ([]byte, error) {
		t.Fatalf("unexpected build call on cache hit")
		return nil, nil
	})
	if err != nil {
		t.Fatalf("Handle(hit) error = %v", err)
	}
	if !fromCache2 {
		t.Fatalf("Handle(hit) fromCache = %t, want %t", fromCache2, true)
	}
	if string(resp2) != "resp" {
		t.Fatalf("resp2 = %q, want %q", string(resp2), "resp")
	}
}

func TestHandledRequestsCache_TTLExpiry(t *testing.T) {
	now := time.Unix(0, 0).UTC()
	clock := func() time.Time { return now }

	c := NewHandledRequestsCache(2, 1*time.Second, clock)

	req := Message{
		Route: Route{
			DstPeerID:       testBase32ID("dst"),
			MsgID:           testBase32ID("req-ttl"),
			CreatedAtUnixMs: 1,
		},
		Signed: Signed{
			SenderPeerID: testBase32ID("sender"),
			Kind:         "echo_request",
			Body:         []byte(`{"n":1}`),
		},
	}

	calls := 0
	build := func() ([]byte, error) {
		calls++
		return []byte("resp"), nil
	}

	_, _, err := c.Handle(req, build)
	if err != nil {
		t.Fatalf("Handle() error = %v", err)
	}

	now = now.Add(2 * time.Second)

	_, fromCache, err := c.Handle(req, build)
	if err != nil {
		t.Fatalf("Handle(after ttl) error = %v", err)
	}
	if fromCache {
		t.Fatalf("Handle(after ttl) fromCache = %t, want %t", fromCache, false)
	}
	if calls != 2 {
		t.Fatalf("calls = %d, want %d", calls, 2)
	}
}

func TestHandledRequestsCache_LRUEviction(t *testing.T) {
	now := time.Unix(0, 0).UTC()
	clock := func() time.Time { return now }

	c := NewHandledRequestsCache(1, 1*time.Hour, clock)

	reqA := Message{
		Route: Route{
			DstPeerID:       testBase32ID("dst"),
			MsgID:           testBase32ID("req-A"),
			CreatedAtUnixMs: 1,
		},
		Signed: Signed{
			SenderPeerID: testBase32ID("sender"),
			Kind:         "echo_request",
			Body:         []byte(`{"n":1}`),
		},
	}
	reqB := reqA
	reqB.Route.MsgID = testBase32ID("req-B")

	callsA := 0
	buildA := func() ([]byte, error) {
		callsA++
		return []byte("respA"), nil
	}
	_, _, _ = c.Handle(reqA, buildA)

	_, _, _ = c.Handle(reqB, func() ([]byte, error) { return []byte("respB"), nil })

	_, fromCache, err := c.Handle(reqA, buildA)
	if err != nil {
		t.Fatalf("Handle(evicted) error = %v", err)
	}
	if fromCache {
		t.Fatalf("Handle(evicted) fromCache = %t, want %t", fromCache, false)
	}
	if callsA != 2 {
		t.Fatalf("callsA = %d, want %d", callsA, 2)
	}
}

func TestHandledRequestsCache_FingerprintMismatch(t *testing.T) {
	now := time.Unix(0, 0).UTC()
	clock := func() time.Time { return now }

	c := NewHandledRequestsCache(2, 1*time.Hour, clock)

	req := Message{
		Route: Route{
			DstPeerID:       testBase32ID("dst"),
			MsgID:           testBase32ID("req-mismatch"),
			CreatedAtUnixMs: 1,
		},
		Signed: Signed{
			SenderPeerID: testBase32ID("sender"),
			Kind:         "echo_request",
			Body:         []byte(`{"n":1}`),
		},
	}

	_, _, _ = c.Handle(req, func() ([]byte, error) { return []byte("resp"), nil })

	calls := 0
	req2 := req
	req2.Signed.Body = []byte(`{"n":2}`)
	_, _, err := c.Handle(req2, func() ([]byte, error) {
		calls++
		return []byte("resp2"), nil
	})
	if !errors.Is(err, ErrRetryInvariantViolation) {
		t.Fatalf("Handle(mismatch) error = %v, want ErrRetryInvariantViolation", err)
	}
	if calls != 0 {
		t.Fatalf("unexpected build calls after mismatch: %d", calls)
	}
}

func TestHandledRequestsCache_ConcurrentSingleflight(t *testing.T) {
	now := time.Unix(0, 0).UTC()
	clock := func() time.Time { return now }

	c := NewHandledRequestsCache(2, 1*time.Hour, clock)

	req := Message{
		Route: Route{
			DstPeerID:       testBase32ID("dst"),
			MsgID:           testBase32ID("req-concurrent"),
			CreatedAtUnixMs: 1,
		},
		Signed: Signed{
			SenderPeerID: testBase32ID("sender"),
			Kind:         "echo_request",
			Body:         []byte(`{"n":1}`),
		},
	}

	started := make(chan struct{})
	unblock := make(chan struct{})

	build := func() ([]byte, error) {
		close(started) // MUST run exactly once.
		<-unblock
		return []byte("resp"), nil
	}

	var (
		resp1, resp2   []byte
		cache1, cache2 bool
		err1, err2     error
	)

	var wg sync.WaitGroup
	wg.Add(2)
	go func() {
		defer wg.Done()
		resp1, cache1, err1 = c.Handle(req, build)
	}()

	<-started

	go func() {
		defer wg.Done()
		resp2, cache2, err2 = c.Handle(req, build)
	}()

	close(unblock)
	wg.Wait()

	if err1 != nil || err2 != nil {
		t.Fatalf("errors: err1=%v err2=%v", err1, err2)
	}
	if cache1 {
		t.Fatalf("cache1 = %t, want %t", cache1, false)
	}
	if !cache2 {
		t.Fatalf("cache2 = %t, want %t", cache2, true)
	}
	if string(resp1) != "resp" || string(resp2) != "resp" {
		t.Fatalf("responses: resp1=%q resp2=%q", string(resp1), string(resp2))
	}
}
