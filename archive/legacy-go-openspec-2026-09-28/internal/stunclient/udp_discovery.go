package stunclient

import (
	"context"
	"errors"
	"fmt"
	"net"
	"strings"
	"sync"
	"time"

	"github.com/pion/stun/v2"

	"github.com/miopunch/miopunch/nat"
)

const defaultUDPResponseTimeout = 3 * time.Second

type DiscoveryResult struct {
	MappedAddrs []string
	Errors      []string
	OkCount     int
	RTTMs       int
}

type StopFunc func(DiscoveryResult) bool

type DiscoverFunc func(ctx context.Context, server string) ([]string, time.Duration, error)

// DiscoverUDP discovers mapped addresses via a sequential UDP STUN strategy.
func DiscoverUDP(ctx context.Context, conn *net.UDPConn, stunServers []string) DiscoveryResult {
	client := NewUDPClient(conn)
	defer client.Close()

	return discoverUDPWithStrategy(ctx, client, stunServers, 1, nil)
}

// DiscoverUDPWithClient discovers mapped addresses using an existing shared UDP client.
func DiscoverUDPWithClient(ctx context.Context, client *UDPClient, stunServers []string, concurrency int, stopFn StopFunc) DiscoveryResult {
	return discoverUDPWithStrategy(ctx, client, stunServers, concurrency, stopFn)
}

func DiscoverFromServerUDP(ctx context.Context, client *UDPClient, addr string) ([]string, time.Duration, error) {
	return discoverFromSTUNServerUDP(ctx, client, addr)
}

func discoverUDPWithStrategy(ctx context.Context, client *UDPClient, stunServers []string, concurrency int, stopFn StopFunc) DiscoveryResult {
	return discoverWithStrategy(ctx, stunServers, concurrency, stopFn, func(reqCtx context.Context, server string) ([]string, time.Duration, error) {
		return discoverFromSTUNServerUDP(reqCtx, client, server)
	})
}

func discoverWithStrategy(ctx context.Context, stunServers []string, concurrency int, stopFn StopFunc, discover DiscoverFunc) DiscoveryResult {
	if concurrency <= 0 {
		concurrency = 1
	}

	res := DiscoveryResult{
		MappedAddrs: make([]string, 0, len(stunServers)*2),
		Errors:      make([]string, 0),
	}
	if len(stunServers) == 0 {
		return res
	}

	runCtx, cancel := context.WithCancel(ctx)
	defer cancel()

	type outcome struct {
		server string
		addrs  []string
		rtt    time.Duration
		err    error
	}

	results := make(chan outcome, concurrency)
	var wg sync.WaitGroup

	next := 0
	active := 0
	stopTriggered := false
	var minRTT time.Duration

	launch := func(server string) {
		active++
		wg.Add(1)
		go func() {
			defer wg.Done()
			addrs, rtt, err := discover(runCtx, server)
			results <- outcome{server: server, addrs: addrs, rtt: rtt, err: err}
		}()
	}

	for active < concurrency && next < len(stunServers) {
		launch(stunServers[next])
		next++
	}

	for active > 0 {
		out := <-results
		active--

		if out.err != nil {
			if !(stopTriggered && errors.Is(out.err, context.Canceled)) {
				res.Errors = append(res.Errors, fmt.Sprintf("%s: %v", out.server, out.err))
			}
		} else {
			res.OkCount++
			if minRTT == 0 || (out.rtt > 0 && out.rtt < minRTT) {
				minRTT = out.rtt
			}
			res.MappedAddrs = append(res.MappedAddrs, out.addrs...)
			if stopFn != nil && stopFn(res) {
				stopTriggered = true
				cancel()
			}
		}

		for !stopTriggered && active < concurrency && next < len(stunServers) {
			launch(stunServers[next])
			next++
		}
	}

	wg.Wait()

	if minRTT > 0 {
		res.RTTMs = int(minRTT.Milliseconds())
	}

	var dropped []string
	res.MappedAddrs, dropped = SanitizeMappedAddrs(res.MappedAddrs)
	if len(dropped) > 0 {
		res.Errors = append(res.Errors, fmt.Sprintf("dropped invalid mapped_addrs: %v", dropped))
	}

	if len(res.MappedAddrs) > 4 {
		res.MappedAddrs = res.MappedAddrs[:4]
	}
	return res
}

func discoverFromSTUNServerUDP(ctx context.Context, client *UDPClient, addr string) ([]string, time.Duration, error) {
	external, other, rtt, err := doSTUNRequestUDP(ctx, client, addr)
	if err != nil {
		return nil, 0, err
	}
	if external == "" {
		return nil, 0, errors.New("no external address found")
	}

	out := make([]string, 0, 2)
	out = append(out, external)
	if other == "" {
		return out, rtt, nil
	}

	external2, _, _, err := doSTUNRequestUDP(ctx, client, other)
	if err != nil {
		return out, rtt, nil
	}
	if external2 != "" {
		out = append(out, external2)
	}
	return out, rtt, nil
}

func doSTUNRequestUDP(ctx context.Context, client *UDPClient, addr string) (externalAddr string, otherAddr string, rtt time.Duration, err error) {
	raddr, err := net.ResolveUDPAddr("udp4", addr)
	if err != nil {
		return "", "", 0, err
	}

	req, err := stun.Build(stun.TransactionID, stun.BindingRequest)
	if err != nil {
		return "", "", 0, err
	}
	if err := req.NewTransactionID(); err != nil {
		return "", "", 0, err
	}

	reqCtx := ctx
	if _, ok := reqCtx.Deadline(); !ok {
		var cancel context.CancelFunc
		reqCtx, cancel = context.WithTimeout(ctx, defaultUDPResponseTimeout)
		defer cancel()
	}

	start := time.Now()
	resp, err := client.RoundTrip(reqCtx, raddr, req)
	if err != nil {
		return "", "", 0, err
	}
	rtt = time.Since(start)

	xor := &stun.XORMappedAddress{}
	mapped := &stun.MappedAddress{}
	changed := &nat.ChangedAddress{}
	other := &stun.OtherAddress{}

	if err := mapped.GetFrom(&resp); err == nil {
		externalAddr = mapped.String()
	}
	if err := xor.GetFrom(&resp); err == nil {
		externalAddr = xor.String()
	}
	if err := changed.GetFrom(&resp); err == nil {
		otherAddr = changed.String()
	}
	if err := other.GetFrom(&resp); err == nil {
		otherAddr = other.String()
	}
	return externalAddr, otherAddr, rtt, nil
}

func SanitizeMappedAddrs(addrs []string) (valid []string, dropped []string) {
	valid = make([]string, 0, len(addrs))
	for _, addr := range addrs {
		addr = strings.TrimSpace(addr)
		if addr == "" {
			dropped = append(dropped, "<empty>")
			continue
		}
		if _, _, err := net.SplitHostPort(addr); err != nil {
			dropped = append(dropped, addr)
			continue
		}
		valid = append(valid, addr)
	}
	return valid, dropped
}
