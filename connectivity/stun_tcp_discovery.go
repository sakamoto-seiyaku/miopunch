package connectivity

import (
	"context"
	"fmt"
	"net"
	"sync"
	"syscall"
	"time"

	"github.com/miopunch/miopunch/internal/stunclient"
)

func DiscoverSTUNTCP(ctx context.Context, dialer *net.Dialer, stunServers []string) STUNDiscoveryResult {
	return discoverSTUNTCPWithStrategy(ctx, dialer, stunServers, 1, nil)
}

func discoverInternalSTUNTCP(ctx context.Context, dialer *net.Dialer, stunServers []string) STUNDiscoveryResult {
	return discoverSTUNTCPWithStrategy(ctx, dialer, stunServers, 1, stunclient.StopFunc(shouldStopInternalSTUNSampling))
}

func discoverSTUNTCPWithStrategy(ctx context.Context, dialer *net.Dialer, stunServers []string, concurrency int, stopFn stunclient.StopFunc) STUNDiscoveryResult {
	if concurrency <= 0 {
		concurrency = 1
	}
	dialer = tcpSTUNDialerWithReuseAddr(dialer)

	out := STUNDiscoveryResult{
		MappedAddrs: make([]string, 0, len(stunServers)),
		Errors:      make([]string, 0),
	}
	if len(stunServers) == 0 {
		return out
	}

	runCtx, cancel := context.WithCancel(ctx)
	defer cancel()

	type outcome struct {
		server string
		addr   string
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
			addr, rtt, err := stunclient.RoundTripTCP(runCtx, dialer, server)
			results <- outcome{server: server, addr: addr, rtt: rtt, err: err}
		}()
	}

	for active < concurrency && next < len(stunServers) {
		launch(stunServers[next])
		next++
	}

	for active > 0 {
		res := <-results
		active--

		if res.err != nil {
			if !(stopTriggered && res.err == context.Canceled) {
				out.Errors = append(out.Errors, fmt.Sprintf("%s: %v", res.server, res.err))
			}
		} else if res.addr != "" {
			out.OkCount++
			if minRTT == 0 || (res.rtt > 0 && res.rtt < minRTT) {
				minRTT = res.rtt
			}
			out.MappedAddrs = append(out.MappedAddrs, res.addr)
			if stopFn != nil {
				stopTriggered = stopFn(stunclient.DiscoveryResult{MappedAddrs: out.MappedAddrs, OkCount: out.OkCount})
				if stopTriggered {
					cancel()
				}
			}
		}

		for !stopTriggered && active < concurrency && next < len(stunServers) {
			launch(stunServers[next])
			next++
		}
	}

	wg.Wait()

	if minRTT > 0 {
		out.RTTMs = int(minRTT.Milliseconds())
	}

	var dropped []string
	out.MappedAddrs, dropped = stunclient.SanitizeMappedAddrs(out.MappedAddrs)
	if len(dropped) > 0 {
		out.Errors = append(out.Errors, fmt.Sprintf("dropped invalid mapped_addrs: %v", dropped))
	}

	if len(out.MappedAddrs) > 4 {
		out.MappedAddrs = out.MappedAddrs[:4]
	}
	return out
}

func tcpSTUNDialerWithReuseAddr(dialer *net.Dialer) *net.Dialer {
	if dialer == nil {
		dialer = &net.Dialer{}
	}

	out := *dialer
	control := out.Control
	controlContext := out.ControlContext
	out.Control = nil
	out.ControlContext = func(ctx context.Context, network, address string, c syscall.RawConn) error {
		if controlContext != nil {
			if err := controlContext(ctx, network, address, c); err != nil {
				return err
			}
		}
		if control != nil {
			if err := control(network, address, c); err != nil {
				return err
			}
		}
		return tcpReuseControl(network, address, c)
	}
	return &out
}
