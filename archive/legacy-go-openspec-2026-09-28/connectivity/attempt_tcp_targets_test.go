package connectivity

import (
	"net/netip"
	"slices"
	"testing"

	"github.com/miopunch/miopunch/internal/wire"
)

func TestBuildTCPPunchTargets_AssistedNotExpanded(t *testing.T) {
	res, err := buildTCPPunchTargets(
		[]string{"203.0.113.10:51100"},
		[]string{"10.0.0.1:5100"},
		[]wire.PortsRange{{From: 51100, To: 51101}},
		0,
	)
	if err != nil {
		t.Fatalf("buildTCPPunchTargets() error = %v, want nil", err)
	}
	if res.AssistedExactCount != 1 {
		t.Fatalf("buildTCPPunchTargets() assisted exact count = %d, want %d", res.AssistedExactCount, 1)
	}
	if res.CandidateExactCount != 1 {
		t.Fatalf("buildTCPPunchTargets() candidate exact count = %d, want %d", res.CandidateExactCount, 1)
	}
	if res.CandidateExpandedCount != 1 {
		t.Fatalf("buildTCPPunchTargets() candidate expanded count = %d, want %d", res.CandidateExpandedCount, 1)
	}

	forbidden := netip.MustParseAddrPort("10.0.0.1:51101")
	if slices.Contains(res.Targets, forbidden) {
		t.Fatalf("buildTCPPunchTargets() targets contains expanded assisted addr %v, want it excluded", forbidden)
	}

	if got := res.Source(netip.MustParseAddrPort("10.0.0.1:5100")); got != "assisted_exact" {
		t.Fatalf("Source(assisted) = %q, want %q", got, "assisted_exact")
	}
	if got := res.Source(netip.MustParseAddrPort("203.0.113.10:51100")); got != "candidate_exact" {
		t.Fatalf("Source(candidate exact) = %q, want %q", got, "candidate_exact")
	}
	if got := res.Source(netip.MustParseAddrPort("203.0.113.10:51101")); got != "candidate_expanded" {
		t.Fatalf("Source(candidate expanded) = %q, want %q", got, "candidate_expanded")
	}
}

func TestBuildTCPPunchTargets_AssistedOnly(t *testing.T) {
	res, err := buildTCPPunchTargets(nil, []string{"10.0.0.1:5100"}, nil, 0)
	if err != nil {
		t.Fatalf("buildTCPPunchTargets(assisted only) error = %v, want nil", err)
	}
	if len(res.Targets) != 1 {
		t.Fatalf("buildTCPPunchTargets(assisted only) targets = %d, want %d", len(res.Targets), 1)
	}
	if res.Targets[0] != netip.MustParseAddrPort("10.0.0.1:5100") {
		t.Fatalf("buildTCPPunchTargets(assisted only) target[0] = %v, want %v", res.Targets[0], "10.0.0.1:5100")
	}
	if res.AssistedExactCount != 1 || res.CandidateExactCount != 0 || res.CandidateExpandedCount != 0 {
		t.Fatalf(
			"buildTCPPunchTargets(assisted only) counts = (assisted=%d exact=%d expanded=%d), want (1,0,0)",
			res.AssistedExactCount,
			res.CandidateExactCount,
			res.CandidateExpandedCount,
		)
	}
}
