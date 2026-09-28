package connectivity

import (
	"net/netip"
	"slices"
	"testing"
)

func TestClassifyTCP4ListenCandidates(t *testing.T) {
	buckets := classifyTCP4ListenCandidates([]string{
		"10.0.0.2",
		"100.64.1.2",
		"203.0.113.2",
		"127.0.0.1",
		"169.254.1.2",
		"not-an-ip",
	}, 5100)

	wantDirect := []netip.AddrPort{netip.MustParseAddrPort("203.0.113.2:5100")}
	if !slices.Equal(buckets.DirectAddrs, wantDirect) {
		t.Fatalf("classifyTCP4ListenCandidates().DirectAddrs = %v, want %v", buckets.DirectAddrs, wantDirect)
	}

	wantAssisted := []string{
		"10.0.0.2:5100",
		"100.64.1.2:5100",
	}
	if !slices.Equal(buckets.AssistedAddrs, wantAssisted) {
		t.Fatalf("classifyTCP4ListenCandidates().AssistedAddrs = %v, want %v", buckets.AssistedAddrs, wantAssisted)
	}

	wantRejected := []string{"127.0.0.1", "169.254.1.2", "not-an-ip"}
	if !slices.Equal(buckets.RejectedSources, wantRejected) {
		t.Fatalf("classifyTCP4ListenCandidates().RejectedSources = %v, want %v", buckets.RejectedSources, wantRejected)
	}
}

func TestFilterTCPPortmapDirectAddrs(t *testing.T) {
	in := []netip.AddrPort{
		netip.MustParseAddrPort("100.64.0.1:5100"), // CGNAT allowed for portmap direct.
		netip.MustParseAddrPort("10.0.0.1:5100"),   // RFC1918 rejected.
		netip.MustParseAddrPort("203.0.113.1:5100"),
		netip.MustParseAddrPort("127.0.0.1:5100"),
	}
	valid, dropped := filterTCPPortmapDirectAddrs(in)

	wantValid := []netip.AddrPort{
		netip.MustParseAddrPort("100.64.0.1:5100"),
		netip.MustParseAddrPort("203.0.113.1:5100"),
	}
	if !slices.Equal(valid, wantValid) {
		t.Fatalf("filterTCPPortmapDirectAddrs() valid = %v, want %v", valid, wantValid)
	}

	wantDropped := []netip.AddrPort{
		netip.MustParseAddrPort("10.0.0.1:5100"),
		netip.MustParseAddrPort("127.0.0.1:5100"),
	}
	if !slices.Equal(dropped, wantDropped) {
		t.Fatalf("filterTCPPortmapDirectAddrs() dropped = %v, want %v", dropped, wantDropped)
	}
}
