package stun

import (
	"context"
	"net"
	"testing"
	"time"

	"github.com/miopunch/miopunch/nat"
)

func freeUDPAddr(t *testing.T) string {
	t.Helper()
	c, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("ListenUDP: %v", err)
	}
	addr := c.LocalAddr().String()
	_ = c.Close()
	return addr
}

func TestServer_Discover(t *testing.T) {
	a1 := freeUDPAddr(t)
	a2 := freeUDPAddr(t)

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	s, err := ListenAndServe(ctx, []string{a1, a2})
	if err != nil {
		t.Fatalf("ListenAndServe: %v", err)
	}
	defer s.Close()

	addrs, local, err := nat.Discover([]string{a1, a2}, "")
	if err != nil {
		t.Fatalf("Discover: %v", err)
	}
	if local == nil {
		t.Fatalf("expected non-nil local addr")
	}
	if len(addrs) < 2 {
		t.Fatalf("expected >=2 addrs, got %d (%v)", len(addrs), addrs)
	}
}
