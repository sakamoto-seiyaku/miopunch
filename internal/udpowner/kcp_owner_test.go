package udpowner

import (
	"bytes"
	"context"
	"net"
	"testing"
	"time"

	kcp "github.com/xtaci/kcp-go/v5"

	"github.com/miopunch/miopunch/internal/netutil"
	"github.com/miopunch/miopunch/internal/punchwire"
	"github.com/miopunch/miopunch/internal/wire"
)

func TestKCPOwner_PacketConnReceivesNonTaggedDatagram(t *testing.T) {
	key := []byte("0123456789abcdef")

	serverConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen udp: %v", err)
	}
	defer serverConn.Close()

	owner, err := NewKCPOwner(serverConn, KCPOwnerConfig{
		Traversal: DemuxConfig{Key: key},
	})
	if err != nil {
		t.Fatalf("NewKCPOwner: %v", err)
	}
	t.Cleanup(func() { _ = owner.Close() })

	peerConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("peer listen udp: %v", err)
	}
	defer peerConn.Close()

	want := []byte{0x01, 0x02, 0x03, 0x04}
	if _, err := peerConn.WriteToUDP(want, serverConn.LocalAddr().(*net.UDPAddr)); err != nil {
		t.Fatalf("WriteToUDP: %v", err)
	}

	pc := owner.PacketConn()
	_ = pc.SetReadDeadline(time.Now().Add(500 * time.Millisecond))

	buf := make([]byte, 2048)
	n, _, err := pc.ReadFrom(buf)
	if err != nil {
		t.Fatalf("ReadFrom: %v", err)
	}
	if !bytes.Equal(buf[:n], want) {
		t.Fatalf("unexpected data: got=%x want=%x", buf[:n], want)
	}
}

func TestKCPOwner_KCPServeConnAcceptsSession(t *testing.T) {
	key := []byte("0123456789abcdef")

	serverConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen udp: %v", err)
	}
	defer serverConn.Close()

	owner, err := NewKCPOwner(serverConn, KCPOwnerConfig{
		Traversal: DemuxConfig{Key: key},
	})
	if err != nil {
		t.Fatalf("NewKCPOwner: %v", err)
	}
	t.Cleanup(func() { _ = owner.Close() })

	ln, err := kcp.ServeConn(nil, 10, 3, owner.PacketConn())
	if err != nil {
		t.Fatalf("kcp.ServeConn: %v", err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	acceptCh := make(chan error, 1)
	go func() {
		_ = ln.SetReadDeadline(time.Now().Add(2 * time.Second))
		sess, err := ln.AcceptKCP()
		if err != nil {
			if sess != nil {
				_ = sess.Close()
			}
			acceptCh <- err
			return
		}

		// Mirror applyKCPDefaults (dataplane) so this test exercises a realistic config.
		sess.SetStreamMode(true)
		sess.SetWriteDelay(true)
		sess.SetNoDelay(1, 20, 2, 1)
		sess.SetMtu(1350)
		sess.SetWindowSize(1024, 1024)
		sess.SetACKNoDelay(false)

		_ = sess.SetReadDeadline(time.Now().Add(1 * time.Second))
		buf := make([]byte, 5)
		_, readErr := sess.Read(buf)
		if readErr != nil {
			_ = sess.Close()
			acceptCh <- readErr
			return
		}

		_ = sess.SetWriteDeadline(time.Now().Add(1 * time.Second))
		_, writeErr := sess.Write([]byte("world"))
		_ = sess.Close()

		if writeErr != nil {
			acceptCh <- writeErr
			return
		}
		acceptCh <- nil
	}()

	clientConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("client listen udp: %v", err)
	}
	defer clientConn.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	kcpConn, err := netutil.NewKCPConnFromUDP(clientConn, false, serverConn.LocalAddr().String())
	if err != nil {
		t.Fatalf("NewKCPConnFromUDP: %v", err)
	}
	defer kcpConn.Close()

	// Trigger initial traffic (KCP has no SYN; the server listener needs to see any packet).
	_ = kcpConn.SetWriteDeadline(time.Now().Add(500 * time.Millisecond))
	_, _ = kcpConn.Write([]byte("hello"))

	_ = kcpConn.SetReadDeadline(time.Now().Add(1 * time.Second))
	reply := make([]byte, 5)
	_, _ = kcpConn.Read(reply)

	select {
	case err := <-acceptCh:
		if err != nil {
			t.Fatalf("AcceptKCP error = %v, want nil", err)
		}
	case <-ctx.Done():
		t.Fatalf("accept timed out: %v", ctx.Err())
	}
}

func TestKCPOwner_BorrowPacketConnCloseDoesNotCloseOwner(t *testing.T) {
	key := []byte("0123456789abcdef")

	serverConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen udp: %v", err)
	}
	owner, err := NewKCPOwner(serverConn, KCPOwnerConfig{
		Traversal: DemuxConfig{Key: key},
	})
	if err != nil {
		t.Fatalf("NewKCPOwner: %v", err)
	}
	t.Cleanup(func() { _ = owner.Close() })

	pc := owner.BorrowPacketConn()
	if err := pc.Close(); err != nil {
		t.Fatalf("BorrowPacketConn().Close() error = %v, want nil", err)
	}
	peerConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("peer listen udp: %v", err)
	}
	t.Cleanup(func() { _ = peerConn.Close() })

	if _, err := peerConn.WriteToUDP([]byte{0x01}, serverConn.LocalAddr().(*net.UDPAddr)); err != nil {
		t.Fatalf("peer WriteToUDP() error = %v, want nil", err)
	}
	freshPC := owner.BorrowPacketConn()
	t.Cleanup(func() { _ = freshPC.Close() })
	_ = freshPC.SetReadDeadline(time.Now().Add(500 * time.Millisecond))
	buf := make([]byte, 16)
	if _, _, err := freshPC.ReadFrom(buf); err != nil {
		t.Fatalf("fresh borrowed PacketConn ReadFrom() error = %v, want nil", err)
	}
}

func TestKCPOwner_TaggedTraversalPacketDoesNotEnterKCPPacketConn(t *testing.T) {
	key := []byte("0123456789abcdef")

	serverConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen udp: %v", err)
	}
	owner, err := NewKCPOwner(serverConn, KCPOwnerConfig{})
	if err != nil {
		t.Fatalf("NewKCPOwner: %v", err)
	}
	t.Cleanup(func() { _ = owner.Close() })

	demux, err := owner.OpenTraversalDemux(DemuxConfig{Key: key})
	if err != nil {
		t.Fatalf("OpenTraversalDemux() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = demux.Close() })
	ep := demux.Open("tx-01", 1)
	t.Cleanup(func() { _ = ep.Close() })

	peerConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("peer listen udp: %v", err)
	}
	t.Cleanup(func() { _ = peerConn.Close() })

	msg := &wire.NatHoleSid{TransactionID: "tx-01", Sid: "sid-01"}
	tagged, err := punchwire.EncodeMessage(msg, key)
	if err != nil {
		t.Fatalf("EncodeMessage() error = %v, want nil", err)
	}
	if _, err := peerConn.WriteToUDP(tagged, serverConn.LocalAddr().(*net.UDPAddr)); err != nil {
		t.Fatalf("peer WriteToUDP(tagged) error = %v, want nil", err)
	}

	recvCtx, cancel := context.WithTimeout(context.Background(), time.Second)
	t.Cleanup(cancel)
	buf := make([]byte, 2048)
	if _, _, err := ep.Recv(recvCtx, buf); err != nil {
		t.Fatalf("TraversalEndpoint.Recv() error = %v, want nil", err)
	}

	pc := owner.BorrowPacketConn()
	t.Cleanup(func() { _ = pc.Close() })
	_ = pc.SetReadDeadline(time.Now().Add(100 * time.Millisecond))
	if _, _, err := pc.ReadFrom(buf); err == nil {
		t.Fatalf("PacketConn.ReadFrom() error = nil, want timeout because tagged packet stays out of KCP")
	}
}
