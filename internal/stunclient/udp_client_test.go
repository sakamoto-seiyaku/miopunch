package stunclient

import (
	"context"
	"net"
	"strconv"
	"sync"
	"testing"
	"time"

	"github.com/pion/stun/v2"
)

func TestUDPClientDispatchesConcurrentResponses(t *testing.T) {
	t.Parallel()

	server1 := startTestUDPSTUNServer(t, "198.51.100.10:41010", 40*time.Millisecond)
	server2 := startTestUDPSTUNServer(t, "198.51.100.20:41020", 5*time.Millisecond)

	conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4zero, Port: 0})
	if err != nil {
		t.Fatalf("ListenUDP() error = %v", err)
	}
	defer conn.Close()

	client := NewUDPClient(conn)
	defer client.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	type result struct {
		addrs []string
		err   error
	}
	results := make(chan result, 2)

	go func() {
		addrs, _, err := discoverFromSTUNServerUDP(ctx, client, server1)
		results <- result{addrs: addrs, err: err}
	}()
	go func() {
		addrs, _, err := discoverFromSTUNServerUDP(ctx, client, server2)
		results <- result{addrs: addrs, err: err}
	}()

	got := make(map[string]struct{}, 2)
	for range 2 {
		res := <-results
		if res.err != nil {
			t.Fatalf("discoverFromSTUNServerUDP() error = %v", res.err)
		}
		if len(res.addrs) != 1 {
			t.Fatalf("addrs = %v, want 1 mapped addr", res.addrs)
		}
		got[res.addrs[0]] = struct{}{}
	}

	for _, want := range []string{"198.51.100.10:41010", "198.51.100.20:41020"} {
		if _, ok := got[want]; !ok {
			t.Fatalf("missing mapped addr %q in %v", want, got)
		}
	}
}

func startTestUDPSTUNServer(t *testing.T, mappedAddr string, delay time.Duration) string {
	t.Helper()

	conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.ParseIP("127.0.0.1"), Port: 0})
	if err != nil {
		t.Fatalf("ListenUDP() error = %v", err)
	}

	host, portText, err := net.SplitHostPort(mappedAddr)
	if err != nil {
		t.Fatalf("SplitHostPort(%q) error = %v", mappedAddr, err)
	}
	port, err := strconv.Atoi(portText)
	if err != nil {
		t.Fatalf("Atoi(%q) error = %v", portText, err)
	}

	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()

		buf := make([]byte, 2048)
		for {
			n, raddr, err := conn.ReadFromUDP(buf)
			if err != nil {
				return
			}
			var req stun.Message
			req.Raw = append(req.Raw[:0], buf[:n]...)
			if err := req.Decode(); err != nil {
				continue
			}

			time.Sleep(delay)
			resp := stun.MustBuild(
				stun.NewTransactionIDSetter(req.TransactionID),
				stun.BindingSuccess,
				&stun.XORMappedAddress{IP: net.ParseIP(host), Port: port},
			)
			_, _ = conn.WriteToUDP(resp.Raw, raddr)
		}
	}()
	t.Cleanup(func() {
		_ = conn.Close()
		wg.Wait()
	})

	return conn.LocalAddr().String()
}
