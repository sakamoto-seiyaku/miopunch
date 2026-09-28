package connectivity

import (
	"cmp"
	"net/netip"
	"slices"
)

const (
	MaxDirectAddrsTotal = 8
	MaxDirectAddrsV6    = 4
	MaxDirectAddrsV4    = 4
)

type ParsedDirectAddrs struct {
	Addrs   []netip.AddrPort
	Invalid []string
}

func ParseDirectAddrPorts(in []string) ParsedDirectAddrs {
	out := ParsedDirectAddrs{
		Addrs:   make([]netip.AddrPort, 0, len(in)),
		Invalid: make([]string, 0),
	}
	for _, raw := range in {
		ap, err := netip.ParseAddrPort(raw)
		if err != nil {
			out.Invalid = append(out.Invalid, raw)
			continue
		}
		out.Addrs = append(out.Addrs, ap)
	}
	return out
}

func SplitAddrPortsByFamily(in []netip.AddrPort) (v6 []netip.AddrPort, v4 []netip.AddrPort) {
	v6 = make([]netip.AddrPort, 0, len(in))
	v4 = make([]netip.AddrPort, 0, len(in))
	for _, ap := range in {
		if ap.Addr().Is6() {
			v6 = append(v6, ap)
			continue
		}
		if ap.Addr().Is4() {
			v4 = append(v4, ap)
		}
	}
	return v6, v4
}

func sortAddrPorts(in []netip.AddrPort) {
	slices.SortFunc(in, func(a, b netip.AddrPort) int {
		if a.Addr().Less(b.Addr()) {
			return -1
		}
		if b.Addr().Less(a.Addr()) {
			return 1
		}
		return cmp.Compare(a.Port(), b.Port())
	})
}

func TrimAndFormatDirectAddrs(in []netip.AddrPort) []string {
	trimmed := TrimDirectAddrPorts(in)
	out := make([]string, 0, len(trimmed))
	for _, ap := range trimmed {
		out = append(out, ap.String())
	}
	return out
}

func TrimDirectAddrPorts(in []netip.AddrPort) []netip.AddrPort {
	if len(in) == 0 {
		return nil
	}

	// De-dup first for stable limits.
	seen := make(map[netip.AddrPort]struct{}, len(in))
	deduped := make([]netip.AddrPort, 0, len(in))
	for _, ap := range in {
		if _, ok := seen[ap]; ok {
			continue
		}
		seen[ap] = struct{}{}
		deduped = append(deduped, ap)
	}

	v6, v4 := SplitAddrPortsByFamily(deduped)
	sortAddrPorts(v6)
	sortAddrPorts(v4)

	if len(v6) > MaxDirectAddrsV6 {
		v6 = v6[:MaxDirectAddrsV6]
	}
	if len(v4) > MaxDirectAddrsV4 {
		v4 = v4[:MaxDirectAddrsV4]
	}

	out := make([]netip.AddrPort, 0, min(MaxDirectAddrsTotal, len(v6)+len(v4)))
	out = append(out, v6...)
	out = append(out, v4...)
	if len(out) > MaxDirectAddrsTotal {
		out = out[:MaxDirectAddrsTotal]
	}
	return out
}
