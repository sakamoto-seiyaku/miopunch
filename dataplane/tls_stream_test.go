package dataplane

import (
	"context"
	"net"
	"testing"
	"time"

	"github.com/miopunch/miopunch/connectivity"
)

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

	errCh := make(chan error, 1)
	go func() {
		errCh <- ServeAndExchangeTLS(ctx, cfg, "sid-1", []byte("secret"), clientCandidates, nil)
	}()

	if err := DialAndExchangeTLS(ctx, cfg, "sid-1", []byte("secret"), visitorCandidates, []byte("ping"), nil); err != nil {
		t.Fatalf("DialAndExchangeTLS() error = %v, want nil", err)
	}

	if err := <-errCh; err != nil {
		t.Fatalf("ServeAndExchangeTLS() error = %v, want nil", err)
	}
}
