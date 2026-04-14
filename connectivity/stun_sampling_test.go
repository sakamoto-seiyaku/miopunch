package connectivity

import (
	"context"
	"net"
	"sync"
	"testing"
	"time"

	"github.com/pion/stun/v2"
)

func TestDiscoverSTUNWithStrategyStopsAfterEnoughObservation(t *testing.T) {
	t.Parallel()

	var (
		mu      sync.Mutex
		started []string
	)

	discover := func(ctx context.Context, server string) ([]string, time.Duration, error) {
		mu.Lock()
		started = append(started, server)
		mu.Unlock()

		switch server {
		case "s1":
			time.Sleep(10 * time.Millisecond)
			return []string{"198.51.100.1:40001"}, 10 * time.Millisecond, nil
		case "s2":
			time.Sleep(20 * time.Millisecond)
			return []string{"198.51.100.2:40002"}, 20 * time.Millisecond, nil
		default:
			<-ctx.Done()
			return nil, 0, ctx.Err()
		}
	}

	res := discoverSTUNWithStrategy(context.Background(), []string{"s1", "s2", "s3", "s4", "s5"}, 2, shouldStopInternalSTUNSampling, discover)
	if res.OkCount != 2 {
		t.Fatalf("OkCount = %d, want 2", res.OkCount)
	}
	if len(res.MappedAddrs) != 2 {
		t.Fatalf("MappedAddrs = %v, want 2 entries", res.MappedAddrs)
	}
	for _, errText := range res.Errors {
		if errText == "s3: context canceled" || errText == "s4: context canceled" || errText == "s5: context canceled" {
			t.Fatalf("unexpected canceled error after stop: %v", res.Errors)
		}
	}

	mu.Lock()
	defer mu.Unlock()
	if len(started) > 3 {
		t.Fatalf("started = %v, want at most initial batch plus one follow-up", started)
	}
	for _, server := range []string{"s4", "s5"} {
		for _, got := range started {
			if got == server {
				t.Fatalf("%s should not have started, got %v", server, started)
			}
		}
	}
}

func TestSharedSTUNClientDispatchesConcurrentResponses(t *testing.T) {
	t.Parallel()

	server1 := startTestSTUNServer(t, "198.51.100.10:41010", 40*time.Millisecond)
	server2 := startTestSTUNServer(t, "198.51.100.20:41020", 5*time.Millisecond)

	conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4zero, Port: 0})
	if err != nil {
		t.Fatalf("ListenUDP() error = %v", err)
	}
	defer conn.Close()

	client := newSharedSTUNClient(conn)
	defer client.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	type result struct {
		addrs []string
		err   error
	}
	results := make(chan result, 2)

	go func() {
		addrs, _, err := discoverFromSTUNServerWithClient(ctx, client, server1)
		results <- result{addrs: addrs, err: err}
	}()
	go func() {
		addrs, _, err := discoverFromSTUNServerWithClient(ctx, client, server2)
		results <- result{addrs: addrs, err: err}
	}()

	got := make(map[string]struct{}, 2)
	for range 2 {
		res := <-results
		if res.err != nil {
			t.Fatalf("discoverFromSTUNServerWithClient() error = %v", res.err)
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

func startTestSTUNServer(t *testing.T, mappedAddr string, delay time.Duration) string {
	t.Helper()

	conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.ParseIP("127.0.0.1"), Port: 0})
	if err != nil {
		t.Fatalf("ListenUDP() error = %v", err)
	}
	t.Cleanup(func() { _ = conn.Close() })

	host, portText, err := net.SplitHostPort(mappedAddr)
	if err != nil {
		t.Fatalf("SplitHostPort(%q) error = %v", mappedAddr, err)
	}
	port, err := net.LookupPort("udp", portText)
	if err != nil {
		t.Fatalf("LookupPort(%q) error = %v", portText, err)
	}

	go func() {
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

	return conn.LocalAddr().String()
}
