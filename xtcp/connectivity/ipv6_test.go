package connectivity

import (
	"net/netip"
	"testing"
)

func TestFilterIPv6Candidates_FiltersObviousBadAddrs(t *testing.T) {
	in := []IPv6IfaceAddr{
		{IfIndex: 1, Prefix: netip.MustParsePrefix("fe80::/64"), Addr: netip.MustParseAddr("fe80::1")}, // link-local
		{IfIndex: 1, Prefix: netip.MustParsePrefix("::/128"), Addr: netip.MustParseAddr("::")},         // unspecified
		{IfIndex: 1, Prefix: netip.MustParsePrefix("ff00::/8"), Addr: netip.MustParseAddr("ff02::1")},  // multicast
	}
	got := FilterIPv6Candidates(in)
	if len(got) != 0 {
		t.Fatalf("unexpected candidates: %v", got)
	}
}

func TestFilterIPv6Candidates_PrefersGlobalOverULA(t *testing.T) {
	in := []IPv6IfaceAddr{
		{IfIndex: 1, Prefix: netip.MustParsePrefix("fc00::/7"), Addr: netip.MustParseAddr("fc00::1")},
		{IfIndex: 1, Prefix: netip.MustParsePrefix("2001:db8::/64"), Addr: netip.MustParseAddr("2001:db8::1")},
	}
	got := FilterIPv6Candidates(in)
	if len(got) != 1 || got[0] != netip.MustParseAddr("2001:db8::1") {
		t.Fatalf("unexpected candidates: %v", got)
	}
}

func TestFilterIPv6Candidates_CapsPerSubnetAndTotal(t *testing.T) {
	in := []IPv6IfaceAddr{
		{IfIndex: 1, Prefix: netip.MustParsePrefix("2001:db8:1::/64"), Addr: netip.MustParseAddr("2001:db8:1::3")},
		{IfIndex: 1, Prefix: netip.MustParsePrefix("2001:db8:1::/64"), Addr: netip.MustParseAddr("2001:db8:1::2")},
		{IfIndex: 1, Prefix: netip.MustParsePrefix("2001:db8:1::/64"), Addr: netip.MustParseAddr("2001:db8:1::1")}, // same subnet (3 items -> cap 2)
		{IfIndex: 1, Prefix: netip.MustParsePrefix("2001:db8:2::/64"), Addr: netip.MustParseAddr("2001:db8:2::1")},
		{IfIndex: 1, Prefix: netip.MustParsePrefix("2001:db8:3::/64"), Addr: netip.MustParseAddr("2001:db8:3::1")},
		{IfIndex: 1, Prefix: netip.MustParsePrefix("2001:db8:4::/64"), Addr: netip.MustParseAddr("2001:db8:4::1")},
		{IfIndex: 1, Prefix: netip.MustParsePrefix("2001:db8:5::/64"), Addr: netip.MustParseAddr("2001:db8:5::1")},
	}

	got := FilterIPv6Candidates(in)
	if len(got) != 4 {
		t.Fatalf("unexpected length: %d (%v)", len(got), got)
	}

	// Sorted; and from the first subnet only two smallest are kept.
	want := []netip.Addr{
		netip.MustParseAddr("2001:db8:1::1"),
		netip.MustParseAddr("2001:db8:1::2"),
		netip.MustParseAddr("2001:db8:2::1"),
		netip.MustParseAddr("2001:db8:3::1"),
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("got[%d]=%v want=%v full=%v", i, got[i], want[i], got)
		}
	}
}
