package connectivity

import (
	"context"
	"net"
	"testing"
	"time"

	"github.com/miopunch/miopunch/xtcp/msg"
)

func TestAttempt_DirectIPv4HandshakeSucceeds(t *testing.T) {
	a, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen A: %v", err)
	}
	defer a.Close()

	b, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen B: %v", err)
	}
	defer b.Close()

	sid := "sid-1"
	key := []byte("0123456789abcdef")

	respA := &msg.NatHoleResp{PeerDirectAddrs: []string{b.LocalAddr().String()}}
	respB := &msg.NatHoleResp{PeerDirectAddrs: []string{a.LocalAddr().String()}}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	cfg := AttemptConfig{
		AttemptPortmapTimeout: 1200 * time.Millisecond,
		DirectSendCount:       3,
		DirectSendInterval:    50 * time.Millisecond,
	}

	type out struct {
		res *AttemptResult
		err error
	}
	ch := make(chan out, 2)
	go func() {
		res, err := Attempt(ctx, sid, key, a, nil, respA, cfg)
		ch <- out{res: res, err: err}
	}()
	go func() {
		res, err := Attempt(ctx, sid, key, b, nil, respB, cfg)
		ch <- out{res: res, err: err}
	}()

	o1 := <-ch
	o2 := <-ch
	if o1.err != nil || o2.err != nil {
		t.Fatalf("attempt errors: o1=%v o2=%v", o1.err, o2.err)
	}
	if o1.res.Path != "direct_ipv4" || o2.res.Path != "direct_ipv4" {
		t.Fatalf("unexpected paths: o1=%v o2=%v", o1.res.Path, o2.res.Path)
	}
	if o1.res.Remote == nil || o2.res.Remote == nil {
		t.Fatalf("unexpected remote nil: o1=%v o2=%v", o1.res.Remote, o2.res.Remote)
	}
}

func TestAttempt_DirectIPv6HandshakeSucceeds(t *testing.T) {
	a, err := net.ListenUDP("udp6", &net.UDPAddr{IP: net.ParseIP("::1"), Port: 0})
	if err != nil {
		t.Fatalf("listen A: %v", err)
	}
	defer a.Close()

	b, err := net.ListenUDP("udp6", &net.UDPAddr{IP: net.ParseIP("::1"), Port: 0})
	if err != nil {
		t.Fatalf("listen B: %v", err)
	}
	defer b.Close()

	sid := "sid-v6"
	key := []byte("0123456789abcdef")

	respA := &msg.NatHoleResp{PeerDirectAddrs: []string{b.LocalAddr().String()}}
	respB := &msg.NatHoleResp{PeerDirectAddrs: []string{a.LocalAddr().String()}}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	cfg := AttemptConfig{
		AttemptV6Timeout:   1200 * time.Millisecond,
		DirectSendCount:    3,
		DirectSendInterval: 50 * time.Millisecond,
	}

	type out struct {
		res *AttemptResult
		err error
	}
	ch := make(chan out, 2)
	go func() {
		res, err := Attempt(ctx, sid, key, a, a, respA, cfg)
		ch <- out{res: res, err: err}
	}()
	go func() {
		res, err := Attempt(ctx, sid, key, b, b, respB, cfg)
		ch <- out{res: res, err: err}
	}()

	o1 := <-ch
	o2 := <-ch
	if o1.err != nil || o2.err != nil {
		t.Fatalf("attempt errors: o1=%v o2=%v", o1.err, o2.err)
	}
	if o1.res.Path != "direct_ipv6" || o2.res.Path != "direct_ipv6" {
		t.Fatalf("unexpected paths: o1=%v o2=%v", o1.res.Path, o2.res.Path)
	}
	if o1.res.Remote == nil || o2.res.Remote == nil {
		t.Fatalf("unexpected remote nil: o1=%v o2=%v", o1.res.Remote, o2.res.Remote)
	}
}

func TestAttempt_PunchingDisabledReturnsError(t *testing.T) {
	conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	defer conn.Close()

	resp := &msg.NatHoleResp{
		PeerDirectAddrs: nil,
		PunchingEnabled: false,
		PunchingError:   "stun missing",
	}

	ctx, cancel := context.WithTimeout(context.Background(), 500*time.Millisecond)
	defer cancel()

	_, err = Attempt(ctx, "sid-2", []byte("0123456789abcdef"), conn, nil, resp, AttemptConfig{})
	if err == nil {
		t.Fatalf("expected error")
	}
}
