package udpowner

import (
	"context"
	"net"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/punchwire"
	"github.com/miopunch/miopunch/internal/wire"
)

func TestUDPTraversalDemux_RoutesTaggedPacketsByTransactionID(t *testing.T) {
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

	demux, err := NewUDPTraversalDemux(listenConn, DemuxConfig{Key: key})
	if err != nil {
		t.Fatalf("NewUDPTraversalDemux: %v", err)
	}
	defer demux.Close()

	ep := demux.Open("tx", 8)
	defer ep.Close()

	want := &wire.NatHoleSid{TransactionID: "tx", Sid: "sid-1", Response: false, Nonce: "0"}
	data, err := punchwire.EncodeMessage(want, key)
	if err != nil {
		t.Fatalf("EncodeMessage: %v", err)
	}
	if _, err := peerConn.WriteToUDP(data, listenConn.LocalAddr().(*net.UDPAddr)); err != nil {
		t.Fatalf("WriteToUDP: %v", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 500*time.Millisecond)
	defer cancel()

	buf := make([]byte, 2048)
	n, _, err := ep.Recv(ctx, buf)
	if err != nil {
		t.Fatalf("Recv: %v", err)
	}

	var got wire.NatHoleSid
	if err := punchwire.DecodeMessageInto(buf[:n], key, &got); err != nil {
		t.Fatalf("DecodeMessageInto: %v", err)
	}
	if got.TransactionID != want.TransactionID || got.Sid != want.Sid || got.Response != want.Response || got.Nonce != want.Nonce {
		t.Fatalf("unexpected decoded message: got=%+v want=%+v", got, *want)
	}
}

func TestUDPTraversalDemux_CloseUnblocksEndpointRecv(t *testing.T) {
	key := []byte("0123456789abcdef")

	listenConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen udp: %v", err)
	}
	defer listenConn.Close()

	demux, err := NewUDPTraversalDemux(listenConn, DemuxConfig{Key: key})
	if err != nil {
		t.Fatalf("NewUDPTraversalDemux: %v", err)
	}

	ep := demux.Open("tx", 1)

	// Closing the demux must unblock any endpoint Recv.
	_ = demux.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 500*time.Millisecond)
	defer cancel()

	buf := make([]byte, 16)
	_, _, recvErr := ep.Recv(ctx, buf)
	if recvErr == nil {
		t.Fatalf("expected Recv error after demux close")
	}
}

func TestKCPOwner_CloseUnblocksPacketConnRead(t *testing.T) {
	key := []byte("0123456789abcdef")

	listenConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen udp: %v", err)
	}
	defer listenConn.Close()

	owner, err := NewKCPOwner(listenConn, KCPOwnerConfig{
		Traversal: DemuxConfig{Key: key},
	})
	if err != nil {
		t.Fatalf("NewKCPOwner: %v", err)
	}

	done := make(chan error, 1)
	go func() {
		pc := owner.PacketConn()
		_ = pc.SetReadDeadline(time.Now().Add(2 * time.Second))
		buf := make([]byte, 16)
		_, _, err := pc.ReadFrom(buf)
		done <- err
	}()

	time.Sleep(50 * time.Millisecond)
	_ = owner.Close()

	select {
	case err := <-done:
		if err == nil {
			t.Fatalf("expected ReadFrom error after owner close")
		}
	case <-time.After(2 * time.Second):
		t.Fatalf("ReadFrom did not unblock after owner close")
	}
}
