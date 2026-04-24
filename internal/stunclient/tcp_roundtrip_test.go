package stunclient

import (
	"context"
	"encoding/binary"
	"io"
	"net"
	"strconv"
	"testing"
	"time"

	"github.com/pion/stun/v2"
)

func TestRoundTripTCP(t *testing.T) {
	t.Parallel()

	server := startTestTCPSTUNServer(t, "198.51.100.30:42030", 10*time.Millisecond)

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	mapped, rtt, err := RoundTripTCP(ctx, nil, server)
	if err != nil {
		t.Fatalf("RoundTripTCP() error = %v", err)
	}
	if mapped != "198.51.100.30:42030" {
		t.Fatalf("RoundTripTCP() mapped = %q, want %q", mapped, "198.51.100.30:42030")
	}
	if rtt <= 0 {
		t.Fatalf("RoundTripTCP() rtt = %v, want >0", rtt)
	}
}

func startTestTCPSTUNServer(t *testing.T, mappedAddr string, delay time.Duration) string {
	t.Helper()

	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("Listen() error = %v", err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	host, portText, err := net.SplitHostPort(mappedAddr)
	if err != nil {
		t.Fatalf("SplitHostPort(%q) error = %v", mappedAddr, err)
	}
	port, err := strconv.Atoi(portText)
	if err != nil {
		t.Fatalf("Atoi(%q) error = %v", portText, err)
	}

	go func() {
		for {
			conn, err := ln.Accept()
			if err != nil {
				return
			}
			go func(c net.Conn) {
				defer c.Close()

				header := make([]byte, 20)
				if _, err := io.ReadFull(c, header); err != nil {
					return
				}
				n := int(binary.BigEndian.Uint16(header[2:4]))
				if n < 0 || n > 4096 {
					return
				}
				body := make([]byte, n)
				if _, err := io.ReadFull(c, body); err != nil {
					return
				}

				var req stun.Message
				req.Raw = append(header, body...)
				if err := req.Decode(); err != nil {
					return
				}

				time.Sleep(delay)
				resp := stun.MustBuild(
					stun.NewTransactionIDSetter(req.TransactionID),
					stun.BindingSuccess,
					&stun.XORMappedAddress{IP: net.ParseIP(host), Port: port},
				)
				_, _ = c.Write(resp.Raw)
			}(conn)
		}
	}()

	return ln.Addr().String()
}
