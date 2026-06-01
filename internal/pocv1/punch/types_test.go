package punch

import (
	"net"
	"testing"
)

func TestPathResultCloseDoesNotCloseBorrowedUDPConn(t *testing.T) {
	t.Parallel()

	conn := mustListenUDPForPathResult(t)
	receiver := mustListenUDPForPathResult(t)

	if err := (PathResult{Conn: conn}).Close(); err != nil {
		t.Fatalf("PathResult.Close() error = %v, want nil", err)
	}
	if _, err := conn.WriteToUDP([]byte("still-open"), receiver.LocalAddr().(*net.UDPAddr)); err != nil {
		t.Fatalf("UDPConn.WriteToUDP() after PathResult.Close() error = %v, want nil", err)
	}
}

func mustListenUDPForPathResult(t *testing.T) *net.UDPConn {
	t.Helper()

	addr, err := net.ResolveUDPAddr("udp4", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("ResolveUDPAddr() error = %v, want nil", err)
	}
	conn, err := net.ListenUDP("udp4", addr)
	if err != nil {
		t.Fatalf("net.ListenUDP() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	return conn
}
