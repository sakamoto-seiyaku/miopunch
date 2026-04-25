package stun

import (
	"context"
	"encoding/binary"
	"errors"
	"io"
	"net"
	"syscall"
	"testing"
	"time"

	"github.com/pion/stun/v2"

	"github.com/miopunch/miopunch/nat"
)

func freeUDPAddr(t *testing.T) string {
	t.Helper()
	c, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		if errors.Is(err, syscall.EPERM) || errors.Is(err, syscall.EACCES) {
			t.Skipf("udp sockets not permitted in this environment: %v", err)
		}
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
		if errors.Is(err, syscall.EPERM) || errors.Is(err, syscall.EACCES) {
			t.Skipf("stun server sockets not permitted in this environment: %v", err)
		}
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

func TestServer_RoundTripTCP(t *testing.T) {
	addr := freeUDPAddr(t)

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	s, err := ListenAndServe(ctx, []string{addr})
	if err != nil {
		if errors.Is(err, syscall.EPERM) || errors.Is(err, syscall.EACCES) {
			t.Skipf("stun server sockets not permitted in this environment: %v", err)
		}
		t.Fatalf("ListenAndServe: %v", err)
	}
	defer s.Close()

	conn, err := (&net.Dialer{}).DialContext(ctx, "tcp", addr)
	if err != nil {
		if errors.Is(err, syscall.EPERM) || errors.Is(err, syscall.EACCES) {
			t.Skipf("tcp sockets not permitted in this environment: %v", err)
		}
		t.Fatalf("DialContext: %v", err)
	}
	defer conn.Close()

	local, ok := conn.LocalAddr().(*net.TCPAddr)
	if !ok {
		t.Fatalf("unexpected local addr type: %T", conn.LocalAddr())
	}

	req, err := stun.Build(stun.TransactionID, stun.BindingRequest)
	if err != nil {
		t.Fatalf("stun.Build: %v", err)
	}
	if err := req.NewTransactionID(); err != nil {
		t.Fatalf("NewTransactionID: %v", err)
	}
	if _, err := conn.Write(req.Raw); err != nil {
		t.Fatalf("Write: %v", err)
	}

	header := make([]byte, 20)
	if _, err := io.ReadFull(conn, header); err != nil {
		t.Fatalf("ReadFull header: %v", err)
	}
	bodyLen := int(binary.BigEndian.Uint16(header[2:4]))
	if bodyLen < 0 || bodyLen > 4096 {
		t.Fatalf("unexpected body length: %d", bodyLen)
	}
	body := make([]byte, bodyLen)
	if _, err := io.ReadFull(conn, body); err != nil {
		t.Fatalf("ReadFull body: %v", err)
	}

	var resp stun.Message
	resp.Raw = append(header, body...)
	if err := resp.Decode(); err != nil {
		t.Fatalf("Decode: %v", err)
	}

	xor := &stun.XORMappedAddress{}
	if err := xor.GetFrom(&resp); err != nil {
		t.Fatalf("GetFrom: %v", err)
	}
	if !xor.IP.Equal(local.IP) {
		t.Fatalf("xor ip = %v, want %v", xor.IP, local.IP)
	}
	if xor.Port != local.Port {
		t.Fatalf("xor port = %v, want %v", xor.Port, local.Port)
	}
}
