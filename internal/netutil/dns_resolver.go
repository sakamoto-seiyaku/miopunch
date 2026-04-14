package netutil

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"math/rand/v2"
	"net"
	"net/netip"
	"strings"
	"time"

	"golang.org/x/net/dns/dnsmessage"
)

type DNSMode string

const (
	DNSModeAuto DNSMode = "auto"
	DNSModeOn   DNSMode = "on"
	DNSModeOff  DNSMode = "off"
)

func ParseDNSMode(value string) (DNSMode, error) {
	value = strings.TrimSpace(strings.ToLower(value))
	switch value {
	case "", "auto":
		return DNSModeAuto, nil
	case "on":
		return DNSModeOn, nil
	case "off":
		return DNSModeOff, nil
	default:
		return "", fmt.Errorf("invalid dns mode: %q", value)
	}
}

func DefaultDNSServers() []string {
	return []string{
		"1.1.1.1:53",
		"8.8.8.8:53",
		"223.5.5.5:53",
		"119.29.29.29:53",
	}
}

type DNSResolver struct {
	mode    DNSMode
	servers []netip.AddrPort
	dialer  net.Dialer
}

func NewDNSResolver(mode string, servers []string) (*DNSResolver, error) {
	parsedMode, err := ParseDNSMode(mode)
	if err != nil {
		return nil, err
	}
	if len(servers) == 0 {
		servers = DefaultDNSServers()
	}

	parsedServers := make([]netip.AddrPort, 0, len(servers))
	for _, s := range servers {
		ap, err := parseDNSServerAddr(s)
		if err != nil {
			return nil, err
		}
		parsedServers = append(parsedServers, ap)
	}

	return &DNSResolver{
		mode:    parsedMode,
		servers: parsedServers,
	}, nil
}

func (r *DNSResolver) LookupNetIP(ctx context.Context, network string, host string) ([]netip.Addr, error) {
	host = strings.TrimSpace(host)
	if host == "" {
		return nil, errors.New("empty host")
	}

	if addr, err := netip.ParseAddr(strings.Trim(host, "[]")); err == nil {
		return []netip.Addr{addr}, nil
	}

	if r == nil {
		return net.DefaultResolver.LookupNetIP(ctx, network, host)
	}

	switch r.mode {
	case DNSModeOff:
		return net.DefaultResolver.LookupNetIP(ctx, network, host)
	case DNSModeOn:
		return r.lookupBuiltin(ctx, network, host)
	case DNSModeAuto:
		addrs, err := net.DefaultResolver.LookupNetIP(ctx, network, host)
		if err == nil && len(addrs) > 0 {
			return addrs, nil
		}
		return r.lookupBuiltin(ctx, network, host)
	default:
		return nil, fmt.Errorf("invalid dns mode: %q", r.mode)
	}
}

func (r *DNSResolver) lookupBuiltin(ctx context.Context, network string, host string) ([]netip.Addr, error) {
	if len(r.servers) == 0 {
		return nil, errors.New("no dns servers configured")
	}

	network = strings.TrimSpace(strings.ToLower(network))
	var qtypes []dnsmessage.Type
	switch network {
	case "ip":
		qtypes = []dnsmessage.Type{dnsmessage.TypeA, dnsmessage.TypeAAAA}
	case "ip4":
		qtypes = []dnsmessage.Type{dnsmessage.TypeA}
	case "ip6":
		qtypes = []dnsmessage.Type{dnsmessage.TypeAAAA}
	default:
		return nil, fmt.Errorf("unsupported dns network: %q", network)
	}

	all := make([]netip.Addr, 0, 4)
	for _, qtype := range qtypes {
		addrs, err := r.lookupQType(ctx, host, qtype)
		if err != nil {
			continue
		}
		all = append(all, addrs...)
	}
	if len(all) == 0 {
		return nil, fmt.Errorf("builtin dns resolution failed for %q", host)
	}
	return all, nil
}

func (r *DNSResolver) lookupQType(ctx context.Context, host string, qtype dnsmessage.Type) ([]netip.Addr, error) {
	fqdn, err := toFQDN(host)
	if err != nil {
		return nil, err
	}

	name, err := dnsmessage.NewName(fqdn)
	if err != nil {
		return nil, err
	}

	cur := name
	for depth := 0; depth < 3; depth++ {
		for _, server := range r.servers {
			addrs, cname, err := r.queryOnce(ctx, server, cur, qtype)
			if err != nil {
				continue
			}
			if len(addrs) > 0 {
				return addrs, nil
			}
			if cname != nil {
				cur = *cname
				goto nextDepth
			}
		}
		break

	nextDepth:
		continue
	}

	return nil, fmt.Errorf("builtin dns query failed for %q type=%v", host, qtype)
}

func (r *DNSResolver) queryOnce(ctx context.Context, server netip.AddrPort, name dnsmessage.Name, qtype dnsmessage.Type) ([]netip.Addr, *dnsmessage.Name, error) {
	id := uint16(rand.Uint32())
	req := dnsmessage.Message{
		Header: dnsmessage.Header{
			ID:                 id,
			RecursionDesired:   true,
			RecursionAvailable: false,
		},
		Questions: []dnsmessage.Question{{
			Name:  name,
			Type:  qtype,
			Class: dnsmessage.ClassINET,
		}},
	}
	payload, err := req.Pack()
	if err != nil {
		return nil, nil, err
	}

	dialCtx := ctx
	if _, ok := dialCtx.Deadline(); !ok {
		var cancel context.CancelFunc
		dialCtx, cancel = context.WithTimeout(ctx, 3*time.Second)
		defer cancel()
	}

	conn, err := r.dialer.DialContext(dialCtx, "tcp", server.String())
	if err != nil {
		return nil, nil, err
	}
	defer conn.Close()

	if deadline, ok := dialCtx.Deadline(); ok {
		_ = conn.SetDeadline(deadline)
	}

	var lenBuf [2]byte
	binary.BigEndian.PutUint16(lenBuf[:], uint16(len(payload)))
	if _, err := conn.Write(lenBuf[:]); err != nil {
		return nil, nil, err
	}
	if _, err := conn.Write(payload); err != nil {
		return nil, nil, err
	}

	if _, err := io.ReadFull(conn, lenBuf[:]); err != nil {
		return nil, nil, err
	}
	n := int(binary.BigEndian.Uint16(lenBuf[:]))
	if n <= 0 || n > 4096 {
		return nil, nil, fmt.Errorf("invalid dns tcp length: %d", n)
	}
	respBuf := make([]byte, n)
	if _, err := io.ReadFull(conn, respBuf); err != nil {
		return nil, nil, err
	}

	var resp dnsmessage.Message
	if err := resp.Unpack(respBuf); err != nil {
		return nil, nil, err
	}
	if resp.Header.ID != id {
		return nil, nil, errors.New("dns id mismatch")
	}
	if !resp.Header.Response {
		return nil, nil, errors.New("dns response bit not set")
	}
	if resp.Header.RCode != dnsmessage.RCodeSuccess {
		return nil, nil, fmt.Errorf("dns rcode=%v", resp.Header.RCode)
	}

	addrs := make([]netip.Addr, 0, 4)
	var cname *dnsmessage.Name
	for _, ans := range resp.Answers {
		switch body := ans.Body.(type) {
		case *dnsmessage.AResource:
			if qtype == dnsmessage.TypeA {
				addrs = append(addrs, netip.AddrFrom4(body.A))
			}
		case *dnsmessage.AAAAResource:
			if qtype == dnsmessage.TypeAAAA {
				addrs = append(addrs, netip.AddrFrom16(body.AAAA))
			}
		case *dnsmessage.CNAMEResource:
			if cname == nil {
				tmp := body.CNAME
				cname = &tmp
			}
		}
	}
	return addrs, cname, nil
}

func parseDNSServerAddr(value string) (netip.AddrPort, error) {
	value = strings.TrimSpace(value)
	if value == "" {
		return netip.AddrPort{}, errors.New("empty dns server")
	}

	if ap, err := netip.ParseAddrPort(value); err == nil {
		return ap, nil
	}
	if addr, err := netip.ParseAddr(value); err == nil {
		return netip.AddrPortFrom(addr, 53), nil
	}

	return netip.AddrPort{}, fmt.Errorf("invalid dns server address: %q", value)
}

func toFQDN(host string) (string, error) {
	host = strings.TrimSpace(host)
	if host == "" {
		return "", errors.New("empty host")
	}
	if strings.HasSuffix(host, ".") {
		return host, nil
	}
	return host + ".", nil
}
