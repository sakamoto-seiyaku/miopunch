package netutil

import (
	"context"
	"encoding/binary"
	"io"
	"net"
	"net/netip"
	"sync"
	"testing"
	"time"

	"golang.org/x/net/dns/dnsmessage"
)

func TestDNSResolver_ModeAuto_FallbackToBuiltin_TCP53(t *testing.T) {
	t.Parallel()

	addr, stop := startTestDNSServer(t)
	defer stop()

	r, err := NewDNSResolver("auto", []string{addr})
	if err != nil {
		t.Fatalf("NewDNSResolver: %v", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	got, err := r.LookupNetIP(ctx, "ip4", "example.invalid")
	if err != nil {
		t.Fatalf("LookupNetIP: %v", err)
	}
	if len(got) == 0 {
		t.Fatalf("expected at least 1 addr")
	}
	if got[0] != netip.MustParseAddr("203.0.113.1") {
		t.Fatalf("unexpected addr: %v", got[0])
	}
}

func TestDNSResolver_ModeOn_UsesBuiltin(t *testing.T) {
	t.Parallel()

	addr, stop := startTestDNSServer(t)
	defer stop()

	r, err := NewDNSResolver("on", []string{addr})
	if err != nil {
		t.Fatalf("NewDNSResolver: %v", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	got, err := r.LookupNetIP(ctx, "ip4", "example.invalid")
	if err != nil {
		t.Fatalf("LookupNetIP: %v", err)
	}
	if len(got) == 0 || got[0] != netip.MustParseAddr("203.0.113.1") {
		t.Fatalf("unexpected addrs: %v", got)
	}
}

func TestDNSResolver_ModeOff_DoesNotUseBuiltin(t *testing.T) {
	t.Parallel()

	addr, stop := startTestDNSServer(t)
	defer stop()

	r, err := NewDNSResolver("off", []string{addr})
	if err != nil {
		t.Fatalf("NewDNSResolver: %v", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	if _, err := r.LookupNetIP(ctx, "ip4", "example.invalid"); err == nil {
		t.Fatalf("expected error in dns mode off")
	}
}

func startTestDNSServer(t *testing.T) (addr string, stop func()) {
	t.Helper()

	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}

	var wg sync.WaitGroup
	stopOnce := sync.Once{}
	stop = func() {
		stopOnce.Do(func() {
			_ = ln.Close()
			wg.Wait()
		})
	}

	wg.Add(1)
	go func() {
		defer wg.Done()
		for {
			conn, err := ln.Accept()
			if err != nil {
				return
			}
			wg.Add(1)
			go func(c net.Conn) {
				defer wg.Done()
				defer c.Close()

				var lenBuf [2]byte
				if _, err := io.ReadFull(c, lenBuf[:]); err != nil {
					return
				}
				n := int(binary.BigEndian.Uint16(lenBuf[:]))
				if n <= 0 || n > 4096 {
					return
				}
				reqBuf := make([]byte, n)
				if _, err := io.ReadFull(c, reqBuf); err != nil {
					return
				}

				var req dnsmessage.Message
				if err := req.Unpack(reqBuf); err != nil {
					return
				}
				if len(req.Questions) == 0 {
					return
				}
				q := req.Questions[0]

				resp := dnsmessage.Message{
					Header: dnsmessage.Header{
						ID:                 req.Header.ID,
						Response:           true,
						RecursionDesired:   req.Header.RecursionDesired,
						RecursionAvailable: true,
					},
					Questions: req.Questions,
				}

				switch q.Type {
				case dnsmessage.TypeA:
					resp.Answers = append(resp.Answers, dnsmessage.Resource{
						Header: dnsmessage.ResourceHeader{
							Name:  q.Name,
							Type:  dnsmessage.TypeA,
							Class: dnsmessage.ClassINET,
							TTL:   60,
						},
						Body: &dnsmessage.AResource{A: [4]byte{203, 0, 113, 1}},
					})
				case dnsmessage.TypeAAAA:
					resp.Answers = append(resp.Answers, dnsmessage.Resource{
						Header: dnsmessage.ResourceHeader{
							Name:  q.Name,
							Type:  dnsmessage.TypeAAAA,
							Class: dnsmessage.ClassINET,
							TTL:   60,
						},
						Body: &dnsmessage.AAAAResource{AAAA: netip.MustParseAddr("2001:db8::1").As16()},
					})
				}

				respPayload, err := resp.Pack()
				if err != nil {
					return
				}
				binary.BigEndian.PutUint16(lenBuf[:], uint16(len(respPayload)))
				_, _ = c.Write(lenBuf[:])
				_, _ = c.Write(respPayload)
			}(conn)
		}
	}()

	return ln.Addr().String(), stop
}
