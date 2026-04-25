package connectivity

import (
	"context"
	"errors"
	"net"
	"strconv"
	"syscall"
	"testing"
	"time"

	"github.com/miopunch/miopunch/stun"
)

func freeUDPAddr(t *testing.T) string {
	t.Helper()

	addrs := freeUDPAddrs(t, 1)
	return addrs[0]
}

func freeUDPAddrs(t *testing.T, count int) []string {
	t.Helper()

	conns := make([]*net.UDPConn, 0, count)
	defer func() {
		for _, c := range conns {
			_ = c.Close()
		}
	}()

	addrs := make([]string, 0, count)
	for range count {
		c, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
		if err != nil {
			if errors.Is(err, syscall.EPERM) || errors.Is(err, syscall.EACCES) {
				t.Skipf("udp sockets not permitted in this environment: %v", err)
			}
			t.Fatalf("ListenUDP: %v", err)
		}
		conns = append(conns, c)
		addrs = append(addrs, c.LocalAddr().String())
	}
	return addrs
}

func TestDiscoverSTUNTCP_BindsLocalPortAcrossEndpoints(t *testing.T) {
	addrs := freeUDPAddrs(t, 2)

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	s, err := stun.ListenAndServe(ctx, addrs)
	if err != nil {
		if errors.Is(err, syscall.EPERM) || errors.Is(err, syscall.EACCES) {
			t.Skipf("stun server sockets not permitted in this environment: %v", err)
		}
		t.Fatalf("ListenAndServe: %v", err)
	}
	defer s.Close()

	ln, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		if errors.Is(err, syscall.EPERM) || errors.Is(err, syscall.EACCES) {
			t.Skipf("tcp sockets not permitted in this environment: %v", err)
		}
		t.Fatalf("ListenTCP: %v", err)
	}
	port := ln.Addr().(*net.TCPAddr).Port
	_ = ln.Close()

	dialer := &net.Dialer{LocalAddr: &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: port}}
	res := DiscoverSTUNTCP(ctx, dialer, addrs)
	if len(res.MappedAddrs) < len(addrs) {
		t.Fatalf("DiscoverSTUNTCP() mapped_addrs = %v, want at least %d", res.MappedAddrs, len(addrs))
	}

	for _, mapped := range res.MappedAddrs {
		ip, portStr, err := net.SplitHostPort(mapped)
		if err != nil {
			t.Fatalf("SplitHostPort(%q): %v", mapped, err)
		}
		gotPort, err := strconv.Atoi(portStr)
		if err != nil {
			t.Fatalf("Atoi(%q): %v", portStr, err)
		}
		if gotPort != port {
			t.Fatalf("mapped port = %d, want %d (mapped=%q)", gotPort, port, mapped)
		}
		if ip != "127.0.0.1" {
			t.Fatalf("mapped ip = %q, want 127.0.0.1 (mapped=%q)", ip, mapped)
		}
	}
}
