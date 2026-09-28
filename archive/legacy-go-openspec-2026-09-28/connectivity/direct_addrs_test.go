package connectivity

import (
	"net/netip"
	"testing"
)

func TestParseDirectAddrPorts_SplitsInvalid(t *testing.T) {
	got := ParseDirectAddrPorts([]string{"[::1]:1234", "bad", "1.2.3.4:80"})
	if len(got.Invalid) != 1 || got.Invalid[0] != "bad" {
		t.Fatalf("unexpected invalid: %#v", got.Invalid)
	}
	if len(got.Addrs) != 2 {
		t.Fatalf("unexpected addrs: %#v", got.Addrs)
	}
}

func TestTrimDirectAddrPorts_DedupSortAndLimits(t *testing.T) {
	in := []netip.AddrPort{
		netip.MustParseAddrPort("203.0.113.3:3"),
		netip.MustParseAddrPort("[2001:db8::2]:2"),
		netip.MustParseAddrPort("[2001:db8::5]:5"),
		netip.MustParseAddrPort("203.0.113.2:2"),
		netip.MustParseAddrPort("[2001:db8::4]:4"),
		netip.MustParseAddrPort("[2001:db8::1]:1"),
		netip.MustParseAddrPort("203.0.113.1:1"),
		netip.MustParseAddrPort("203.0.113.4:4"),
		netip.MustParseAddrPort("203.0.113.5:5"),
		netip.MustParseAddrPort("[2001:db8::3]:3"),
		netip.MustParseAddrPort("[2001:db8::3]:3"), // dup
	}

	got := TrimDirectAddrPorts(in)
	if len(got) != 8 {
		t.Fatalf("unexpected length: %d (%v)", len(got), got)
	}

	// v6 first, sorted, capped at 4.
	wantPrefix := []netip.AddrPort{
		netip.MustParseAddrPort("[2001:db8::1]:1"),
		netip.MustParseAddrPort("[2001:db8::2]:2"),
		netip.MustParseAddrPort("[2001:db8::3]:3"),
		netip.MustParseAddrPort("[2001:db8::4]:4"),
	}
	for i := range wantPrefix {
		if got[i] != wantPrefix[i] {
			t.Fatalf("unexpected v6[%d]: got=%v want=%v full=%v", i, got[i], wantPrefix[i], got)
		}
	}

	// v4 after v6, sorted, capped at 4.
	wantSuffix := []netip.AddrPort{
		netip.MustParseAddrPort("203.0.113.1:1"),
		netip.MustParseAddrPort("203.0.113.2:2"),
		netip.MustParseAddrPort("203.0.113.3:3"),
		netip.MustParseAddrPort("203.0.113.4:4"),
	}
	for i := range wantSuffix {
		idx := len(wantPrefix) + i
		if got[idx] != wantSuffix[i] {
			t.Fatalf("unexpected v4[%d]: got=%v want=%v full=%v", i, got[idx], wantSuffix[i], got)
		}
	}
}
