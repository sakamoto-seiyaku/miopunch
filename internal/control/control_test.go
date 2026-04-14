package control

import (
	"context"
	"io"
	"net"
	"testing"
	"time"
)

func TestDialListen_TCP(t *testing.T)  { testDialListen(t, ProtoTCP) }
func TestDialListen_KCP(t *testing.T)  { testDialListen(t, ProtoKCP) }
func TestDialListen_QUIC(t *testing.T) { testDialListen(t, ProtoQUIC) }

func testDialListen(t *testing.T, proto Protocol) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	l, err := Listen("127.0.0.1:0", proto)
	if err != nil {
		t.Fatalf("Listen(%s): %v", proto, err)
	}
	defer l.Close()

	_, port, err := net.SplitHostPort(l.Addr().String())
	if err != nil {
		t.Fatalf("SplitHostPort: %v", err)
	}
	addr := net.JoinHostPort("127.0.0.1", port)

	go func() {
		<-ctx.Done()
		_ = l.Close()
	}()
	serverErr := make(chan error, 1)
	go func() {
		c, err := l.Accept(ctx)
		if err != nil {
			serverErr <- err
			return
		}
		defer c.Close()
		if nc, ok := c.(interface{ SetDeadline(time.Time) error }); ok {
			if dl, ok := ctx.Deadline(); ok {
				_ = nc.SetDeadline(dl)
			}
		}
		buf := make([]byte, 3)
		if _, err := io.ReadFull(c, buf); err != nil {
			serverErr <- err
			return
		}
		if _, err := c.Write(buf); err != nil {
			serverErr <- err
			return
		}
		serverErr <- nil
	}()

	c, err := Dial(ctx, addr, proto)
	if err != nil {
		t.Fatalf("Dial(%s): %v", proto, err)
	}
	t.Cleanup(func() {
		if c != nil {
			_ = c.Close()
		}
	})

	var quicRWC *quicStreamRWC
	if proto == ProtoQUIC {
		var ok bool
		quicRWC, ok = c.(*quicStreamRWC)
		if !ok {
			t.Fatalf("Dial(%s): unexpected type %T", proto, c)
		}
	}
	if nc, ok := c.(interface{ SetDeadline(time.Time) error }); ok {
		if dl, ok := ctx.Deadline(); ok {
			_ = nc.SetDeadline(dl)
		}
	}

	if _, err := c.Write([]byte("hey")); err != nil {
		t.Fatalf("client write: %v", err)
	}
	got := make([]byte, 3)
	if _, err := io.ReadFull(c, got); err != nil {
		t.Fatalf("client read: %v", err)
	}
	if string(got) != "hey" {
		t.Fatalf("unexpected echo: %q", string(got))
	}

	if proto == ProtoQUIC {
		if err := c.Close(); err != nil {
			t.Fatalf("client close: %v", err)
		}
		c = nil

		select {
		case <-quicRWC.conn.Context().Done():
		case <-time.After(1 * time.Second):
			t.Fatalf("expected quic conn context done after close")
		}
	}
	select {
	case err := <-serverErr:
		if err != nil {
			t.Fatalf("server: %v", err)
		}
	case <-ctx.Done():
		t.Fatalf("timeout waiting for server")
	}
}
