package connectivity

import (
	"context"
	"errors"
	"net/netip"
	"testing"
	"time"
)

func TestRunPortMap_InvalidInternalPort(t *testing.T) {
	res, _ := runPortMap(context.Background(), PortMapOptions{InternalPort: 0}, nil)
	if len(res.Attempts) != 1 || res.Attempts[0].Err == nil {
		t.Fatalf("expected error attempt, got %#v", res.Attempts)
	}
}

func TestRunPortMap_DedupAndTrimCandidates(t *testing.T) {
	m1 := func(context.Context, int, time.Duration) (PortMapAttemptResult, PortMapCleanup) {
		return PortMapAttemptResult{
			Method: "m1",
			Candidates: []netip.AddrPort{
				netip.MustParseAddrPort("203.0.113.1:1"),
				netip.MustParseAddrPort("203.0.113.2:2"),
				netip.MustParseAddrPort("203.0.113.3:3"),
				netip.MustParseAddrPort("203.0.113.4:4"),
				netip.MustParseAddrPort("203.0.113.5:5"), // should be trimmed (v4 max=4)
			},
		}, nil
	}
	m2 := func(context.Context, int, time.Duration) (PortMapAttemptResult, PortMapCleanup) {
		return PortMapAttemptResult{
			Method: "m2",
			Candidates: []netip.AddrPort{
				netip.MustParseAddrPort("203.0.113.2:2"), // dup
			},
		}, nil
	}

	res, _ := runPortMap(context.Background(), PortMapOptions{InternalPort: 1234, Lease: time.Minute}, []portMapperFunc{m1, m2})
	if len(res.Candidates) != 4 {
		t.Fatalf("unexpected candidates: %v", res.Candidates)
	}
}

func TestRunPortMap_CleanupAggregatesErrors(t *testing.T) {
	var called int
	m1 := func(context.Context, int, time.Duration) (PortMapAttemptResult, PortMapCleanup) {
		return PortMapAttemptResult{Method: "m1"}, func(context.Context) error {
			called++
			return errors.New("e1")
		}
	}
	m2 := func(context.Context, int, time.Duration) (PortMapAttemptResult, PortMapCleanup) {
		return PortMapAttemptResult{Method: "m2"}, func(context.Context) error {
			called++
			return nil
		}
	}

	_, cleanup := runPortMap(context.Background(), PortMapOptions{InternalPort: 1234, Lease: time.Minute}, []portMapperFunc{m1, m2})
	err := cleanup(context.Background())
	if err == nil {
		t.Fatalf("expected error")
	}
	if called != 2 {
		t.Fatalf("expected cleanup called twice, got %d", called)
	}
}
