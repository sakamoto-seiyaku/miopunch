package punchdecision

import (
	"math"

	"github.com/miopunch/miopunch/internal/wire"
)

const stunRTTTieThresholdMs = 30

type stunViewAggregate struct {
	name          string
	available     bool
	natDifficulty int
	rttMs         int
	okCount       int
}

func aggregateSTUNView(name string, visitorObs, clientObs *wire.STUNViewObservation) stunViewAggregate {
	out := stunViewAggregate{
		name:          name,
		available:     false,
		natDifficulty: 999,
		rttMs:         999999,
		okCount:       0,
	}
	if visitorObs == nil || clientObs == nil {
		return out
	}

	out.available = visitorObs.Available && clientObs.Available
	out.natDifficulty = visitorObs.NATDifficulty + clientObs.NATDifficulty
	out.rttMs = visitorObs.RTTMs + clientObs.RTTMs
	out.okCount = visitorObs.OkCount + clientObs.OkCount
	return out
}

// selectSTUNView deterministically selects exactly one view out of cn/global
// based on the spec-defined order:
// availability → NAT difficulty → STUN RTT (30ms tie) → ok_count → default global.
func selectSTUNView(cn, global stunViewAggregate) (selectedView string, reason string) {
	// 1) availability
	if cn.available != global.available {
		if global.available {
			return global.name, "availability"
		}
		return cn.name, "availability"
	}

	// 2) NAT difficulty (smaller is easier)
	if cn.natDifficulty != global.natDifficulty {
		if global.natDifficulty < cn.natDifficulty {
			return global.name, "nat_difficulty"
		}
		return cn.name, "nat_difficulty"
	}

	// 3) STUN RTT (smaller is better, only when NAT difficulty ties)
	if absInt(cn.rttMs-global.rttMs) > stunRTTTieThresholdMs {
		if global.rttMs < cn.rttMs {
			return global.name, "stun_rtt"
		}
		return cn.name, "stun_rtt"
	}

	// 4) ok_count (larger is better)
	if cn.okCount != global.okCount {
		if global.okCount > cn.okCount {
			return global.name, "ok_count"
		}
		return cn.name, "ok_count"
	}

	// 5) hard tie defaults to global.
	return global.name, "default_global"
}

func absInt(v int) int {
	return int(math.Abs(float64(v)))
}
