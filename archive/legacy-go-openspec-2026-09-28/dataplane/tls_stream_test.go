package dataplane

import (
	"bytes"
	"context"
	"net"
	"sync"
	"testing"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/event"
)

func TestConvergePinnedTLS_ConfigFailureClosesCandidates(t *testing.T) {
	conn, peer := net.Pipe()
	defer peer.Close()

	tracked := &closeTrackingConn{Conn: conn}
	_, err := convergePinnedTLS(
		context.Background(),
		"",
		[]byte("secret"),
		tlsRoleVisitor,
		tlsRoleClient,
		true,
		[]connectivity.TCPConn{{Conn: tracked, Origin: connectivity.TCPConnOriginDial}},
		nil,
	)
	if err == nil {
		t.Fatalf("convergePinnedTLS(empty sid) error = nil, want error")
	}
	if !tracked.closed() {
		t.Fatalf("convergePinnedTLS(empty sid) did not close candidate connection")
	}
}

func TestTLSStream_ConvergesAndExchangesWithMultipleCandidates(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	v1, c1 := net.Pipe()
	v2, c2 := net.Pipe()

	visitorCandidates := []connectivity.TCPConn{
		{Conn: v1, Origin: connectivity.TCPConnOriginDial},
		{Conn: v2, Origin: connectivity.TCPConnOriginDial},
	}
	clientCandidates := []connectivity.TCPConn{
		{Conn: c1, Origin: connectivity.TCPConnOriginAccept},
		{Conn: c2, Origin: connectivity.TCPConnOriginAccept},
	}

	cfg := Config{Proto: ProtocolTLS}

	var dialEvents bytes.Buffer
	var serveEvents bytes.Buffer
	dialEm := event.NewEmitter(&dialEvents, "dial")
	serveEm := event.NewEmitter(&serveEvents, "serve")

	errCh := make(chan error, 1)
	go func() {
		errCh <- ServeAndExchangeTLS(ctx, cfg, "sid-1", []byte("secret"), clientCandidates, serveEm)
	}()

	if err := DialAndExchangeTLS(ctx, cfg, "sid-1", []byte("secret"), visitorCandidates, []byte("ping"), dialEm); err != nil {
		t.Fatalf("DialAndExchangeTLS() error = %v, want nil\n--- dial events ---\n%s\n--- serve events ---\n%s", err, dialEvents.String(), serveEvents.String())
	}

	if err := <-errCh; err != nil {
		t.Fatalf("ServeAndExchangeTLS() error = %v, want nil\n--- dial events ---\n%s\n--- serve events ---\n%s", err, dialEvents.String(), serveEvents.String())
	}
}

func TestTLSStream_ConvergesBeforeSlowCandidateFails(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	visitorFast, clientFast := net.Pipe()
	visitorSlow, slowPeer := net.Pipe()
	defer slowPeer.Close()

	visitorCandidates := []connectivity.TCPConn{
		{Conn: visitorFast, Origin: connectivity.TCPConnOriginDial},
		{Conn: visitorSlow, Origin: connectivity.TCPConnOriginDial},
	}
	clientCandidates := []connectivity.TCPConn{
		{Conn: clientFast, Origin: connectivity.TCPConnOriginAccept},
	}

	cfg := Config{Proto: ProtocolTLS}
	var dialEvents bytes.Buffer
	var serveEvents bytes.Buffer
	dialEm := event.NewEmitter(&dialEvents, "dial")
	serveEm := event.NewEmitter(&serveEvents, "serve")

	errCh := make(chan error, 1)
	go func() {
		errCh <- ServeAndExchangeTLS(ctx, cfg, "sid-fast-settle", []byte("secret"), clientCandidates, serveEm)
	}()

	if err := DialAndExchangeTLS(ctx, cfg, "sid-fast-settle", []byte("secret"), visitorCandidates, []byte("ping"), dialEm); err != nil {
		t.Fatalf("DialAndExchangeTLS() error = %v, want nil\n--- dial events ---\n%s\n--- serve events ---\n%s", err, dialEvents.String(), serveEvents.String())
	}

	if err := <-errCh; err != nil {
		t.Fatalf("ServeAndExchangeTLS() error = %v, want nil\n--- dial events ---\n%s\n--- serve events ---\n%s", err, dialEvents.String(), serveEvents.String())
	}
}

type closeTrackingConn struct {
	net.Conn

	mu       sync.Mutex
	isClosed bool
}

func (c *closeTrackingConn) Close() error {
	c.mu.Lock()
	c.isClosed = true
	c.mu.Unlock()
	return c.Conn.Close()
}

func (c *closeTrackingConn) closed() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.isClosed
}
