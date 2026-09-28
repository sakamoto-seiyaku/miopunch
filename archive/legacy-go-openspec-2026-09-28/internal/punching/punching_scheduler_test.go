package punching

import (
	"context"
	"errors"
	"net"
	"sync"
	"testing"
	"time"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/eventctx"
	"github.com/miopunch/miopunch/internal/udpowner"
	"github.com/miopunch/miopunch/internal/wire"
)

type eventRecorder struct {
	mu    sync.Mutex
	names []string
}

func (r *eventRecorder) Emit(ev event.Event) {
	r.mu.Lock()
	r.names = append(r.names, ev.Name)
	r.mu.Unlock()
}

func (r *eventRecorder) Names() []string {
	r.mu.Lock()
	defer r.mu.Unlock()
	return append([]string(nil), r.names...)
}

func indexOf(xs []string, v string) int {
	for i := range xs {
		if xs[i] == v {
			return i
		}
	}
	return -1
}

func TestMakeHole_ReceiveBeforeProbe_CanWinDuringDelay(t *testing.T) {
	key := []byte("0123456789abcdef")

	listenConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen udp: %v", err)
	}
	defer listenConn.Close()

	peerConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("peer listen udp: %v", err)
	}
	defer peerConn.Close()

	resp := &wire.NatHoleResp{
		TransactionID: "local-control-tx",
		Sid:           "sid-1",
		DetectBehavior: wire.NatHoleDetectBehavior{
			Role:          DetectRoleReceiver,
			Mode:          0,
			TTL:           0,
			SendDelayMs:   500,
			ReadTimeoutMs: 500,
		},
		CandidateAddrs: []string{listenConn.LocalAddr().String()},
	}

	rec := &eventRecorder{}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	ctx = eventctx.WithEmitFunc(ctx, rec.Emit)

	demux, err := udpowner.NewUDPTraversalDemux(listenConn, udpowner.DemuxConfig{Key: key})
	if err != nil {
		t.Fatalf("NewUDPTraversalDemux error: %v", err)
	}
	defer demux.Close()

	go func() {
		time.Sleep(50 * time.Millisecond)
		msg := &wire.NatHoleSid{TransactionID: resp.Sid, Sid: resp.Sid, Response: false, Nonce: "0"}
		data, err := EncodeMessage(msg, key)
		if err != nil {
			return
		}
		_, _ = peerConn.WriteToUDP(data, listenConn.LocalAddr().(*net.UDPAddr))
	}()

	gotConn, gotAddr, err := MakeHole(ctx, listenConn, demux, resp, key)
	if err != nil {
		t.Fatalf("MakeHole error: %v", err)
	}
	if gotConn == nil || gotAddr == nil {
		t.Fatalf("MakeHole returned nil conn/addr: conn=%v addr=%v", gotConn, gotAddr)
	}

	// Winner conn must still be usable (regression test for accidental close).
	if err := gotConn.SetReadDeadline(time.Now().Add(10 * time.Millisecond)); err != nil {
		t.Fatalf("winner conn already closed: %v", err)
	}

	names := rec.Names()
	if idx := indexOf(names, "attempt.punching.recv.start"); idx < 0 {
		t.Fatalf("missing recv.start event, got %v", names)
	}
	if idx := indexOf(names, "attempt.punching.winner"); idx < 0 {
		t.Fatalf("missing winner event, got %v", names)
	}
	if idx := indexOf(names, "attempt.punching.probe.start"); idx >= 0 {
		t.Fatalf("unexpected probe.start during delay winner, got %v", names)
	}
}

func TestMakeHole_CancelBeforeProbe_EmitsCanceledAndReturnsContextError(t *testing.T) {
	key := []byte("0123456789abcdef")

	listenConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen udp: %v", err)
	}
	defer listenConn.Close()

	resp := &wire.NatHoleResp{
		TransactionID: "tx",
		Sid:           "sid-2",
		DetectBehavior: wire.NatHoleDetectBehavior{
			Role:          DetectRoleSender,
			Mode:          0,
			TTL:           0,
			SendDelayMs:   500,
			ReadTimeoutMs: 500,
		},
		CandidateAddrs: []string{listenConn.LocalAddr().String()},
	}

	rec := &eventRecorder{}
	ctx, cancel := context.WithCancel(context.Background())
	ctx = eventctx.WithEmitFunc(ctx, rec.Emit)

	demux, err := udpowner.NewUDPTraversalDemux(listenConn, udpowner.DemuxConfig{Key: key})
	if err != nil {
		t.Fatalf("NewUDPTraversalDemux error: %v", err)
	}
	defer demux.Close()

	go func() {
		time.Sleep(50 * time.Millisecond)
		cancel()
	}()

	_, _, err = MakeHole(ctx, listenConn, demux, resp, key)
	if err == nil {
		t.Fatalf("expected error")
	}
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("expected context.Canceled, got %v", err)
	}

	names := rec.Names()
	if idx := indexOf(names, "attempt.punching.canceled"); idx < 0 {
		t.Fatalf("missing canceled event, got %v", names)
	}
	if idx := indexOf(names, "attempt.punching.timeout"); idx >= 0 {
		t.Fatalf("unexpected timeout event on cancel, got %v", names)
	}
}

func TestMakeHole_ListenRandomPortsWinnerStaysOpen(t *testing.T) {
	key := []byte("0123456789abcdef")

	listenConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen udp: %v", err)
	}
	defer listenConn.Close()

	peerConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("peer listen udp: %v", err)
	}
	defer peerConn.Close()

	resp := &wire.NatHoleResp{
		TransactionID: "tx-random-listen",
		Sid:           "sid-random-listen",
		DetectBehavior: wire.NatHoleDetectBehavior{
			Role:              DetectRoleSender,
			Mode:              2,
			TTL:               0,
			ReadTimeoutMs:     1000,
			ListenRandomPorts: 2,
		},
		CandidateAddrs: []string{peerConn.LocalAddr().String()},
	}

	primaryAddr := listenConn.LocalAddr().String()
	respondedFrom := make(chan string, 1)
	go func() {
		buf := make([]byte, 2048)
		_ = peerConn.SetReadDeadline(time.Now().Add(2 * time.Second))
		for {
			n, raddr, err := peerConn.ReadFromUDP(buf)
			if err != nil {
				return
			}
			if raddr.String() == primaryAddr {
				continue
			}
			var msg wire.NatHoleSid
			if err := DecodeMessageInto(buf[:n], key, &msg); err != nil {
				continue
			}
			if msg.Sid != resp.Sid {
				continue
			}
			msg.Response = true
			out, err := EncodeMessage(&msg, key)
			if err != nil {
				return
			}
			_, _ = peerConn.WriteToUDP(out, raddr)
			respondedFrom <- raddr.String()
			return
		}
	}()

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	demux, err := udpowner.NewUDPTraversalDemux(listenConn, udpowner.DemuxConfig{Key: key})
	if err != nil {
		t.Fatalf("NewUDPTraversalDemux error: %v", err)
	}
	defer demux.Close()

	gotConn, gotAddr, err := MakeHole(ctx, listenConn, demux, resp, key)
	if err != nil {
		t.Fatalf("MakeHole error: %v", err)
	}
	if gotConn == nil || gotAddr == nil {
		t.Fatalf("MakeHole returned nil conn/addr: conn=%v addr=%v", gotConn, gotAddr)
	}
	if gotConn == listenConn {
		t.Fatalf("MakeHole winner conn = primary conn, want random listener")
	}
	if gotConn.LocalAddr().String() == primaryAddr {
		t.Fatalf("MakeHole winner addr = %s, want non-primary", gotConn.LocalAddr())
	}
	if gotAddr.String() != peerConn.LocalAddr().String() {
		t.Fatalf("MakeHole remote addr = %s, want %s", gotAddr, peerConn.LocalAddr())
	}
	gotResponseAddr := <-respondedFrom
	responseUDPAddr, err := net.ResolveUDPAddr("udp", gotResponseAddr)
	if err != nil {
		t.Fatalf("ResolveUDPAddr(%q) error: %v", gotResponseAddr, err)
	}
	winnerUDPAddr := gotConn.LocalAddr().(*net.UDPAddr)
	if responseUDPAddr.Port != winnerUDPAddr.Port {
		t.Fatalf("peer responded to %s, winner local addr = %s", gotResponseAddr, gotConn.LocalAddr())
	}
	if err := gotConn.SetReadDeadline(time.Now().Add(10 * time.Millisecond)); err != nil {
		t.Fatalf("random winner conn was closed: %v", err)
	}
	if err := listenConn.SetReadDeadline(time.Now().Add(10 * time.Millisecond)); err != nil {
		t.Fatalf("primary conn was closed: %v", err)
	}
	_ = gotConn.Close()
}
