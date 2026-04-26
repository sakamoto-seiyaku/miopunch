package punchdecision

import (
	"testing"

	"github.com/miopunch/miopunch/internal/wire"
)

func TestSelectSTUNView_AvailabilityFirst(t *testing.T) {
	cn := aggregateSTUNView("cn",
		&wire.STUNViewObservation{Available: false},
		&wire.STUNViewObservation{Available: false},
	)
	global := aggregateSTUNView("global",
		&wire.STUNViewObservation{Available: true, NATDifficulty: 10, RTTMs: 999, OkCount: 1},
		&wire.STUNViewObservation{Available: true, NATDifficulty: 10, RTTMs: 999, OkCount: 1},
	)

	view, reason := selectSTUNView(cn, global)
	if view != "global" || reason != "availability" {
		t.Fatalf("expected global/availability, got %s/%s", view, reason)
	}
}

func TestSelectSTUNView_NATDifficultyBeatsRTT(t *testing.T) {
	cn := aggregateSTUNView("cn",
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 10, OkCount: 1},
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 10, OkCount: 1},
	)
	global := aggregateSTUNView("global",
		&wire.STUNViewObservation{Available: true, NATDifficulty: 0, RTTMs: 9999, OkCount: 1},
		&wire.STUNViewObservation{Available: true, NATDifficulty: 0, RTTMs: 9999, OkCount: 1},
	)

	view, reason := selectSTUNView(cn, global)
	if view != "global" || reason != "nat_difficulty" {
		t.Fatalf("expected global/nat_difficulty, got %s/%s", view, reason)
	}
}

func TestSelectSTUNView_RTTWhenDifficultyTies(t *testing.T) {
	cn := aggregateSTUNView("cn",
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 100, OkCount: 1},
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 100, OkCount: 1},
	)
	global := aggregateSTUNView("global",
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 10, OkCount: 1},
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 10, OkCount: 1},
	)

	view, reason := selectSTUNView(cn, global)
	if view != "global" || reason != "stun_rtt" {
		t.Fatalf("expected global/stun_rtt, got %s/%s", view, reason)
	}
}

func TestSelectSTUNView_RTTTieUsesOkCount(t *testing.T) {
	cn := aggregateSTUNView("cn",
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 100, OkCount: 10},
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 100, OkCount: 10},
	)
	// RTT diff is 20ms (< 30ms tie threshold), so ok_count decides.
	global := aggregateSTUNView("global",
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 110, OkCount: 11},
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 110, OkCount: 11},
	)

	view, reason := selectSTUNView(cn, global)
	if view != "global" || reason != "ok_count" {
		t.Fatalf("expected global/ok_count, got %s/%s", view, reason)
	}
}

func TestSelectSTUNView_HardTieDefaultsToGlobal(t *testing.T) {
	cn := aggregateSTUNView("cn",
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 100, OkCount: 2},
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 100, OkCount: 2},
	)
	global := aggregateSTUNView("global",
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 110, OkCount: 2},
		&wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 110, OkCount: 2},
	)

	view, reason := selectSTUNView(cn, global)
	if view != "global" || reason != "default_global" {
		t.Fatalf("expected global/default_global, got %s/%s", view, reason)
	}

	for i := 0; i < 10; i++ {
		againView, againReason := selectSTUNView(cn, global)
		if againView != view || againReason != reason {
			t.Fatalf("selection not deterministic: got %s/%s on iter %d", againView, againReason, i)
		}
	}
}
