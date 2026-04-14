package connectivity

import (
	"context"
	"errors"
	"fmt"
	"net"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/pion/stun/v2"

	"github.com/miopunch/miopunch/nat"
)

const stunResponseTimeout = 3 * time.Second

const (
	internalSTUNMaxConcurrency        = 3
	internalSTUNResolvedEndpointLimit = internalSTUNMaxConcurrency * 2
	internalSTUNMinMappedAddrs        = 2
	internalSTUNPreferredAddrs        = 3
	stunReadLoopPollInterval          = 100 * time.Millisecond
)

type STUNDiscoveryResult struct {
	MappedAddrs []string
	Errors      []string
	OkCount     int
	RTTMs       int
}

func DiscoverSTUN(ctx context.Context, conn *net.UDPConn, stunServers []string) STUNDiscoveryResult {
	return discoverSTUNWithStrategy(ctx, stunServers, 1, nil, func(reqCtx context.Context, server string) ([]string, time.Duration, error) {
		return discoverFromSTUNServer(reqCtx, conn, server)
	})
}

func discoverInternalSTUN(ctx context.Context, client *sharedSTUNClient, stunServers []string) STUNDiscoveryResult {
	return discoverSTUNWithStrategy(ctx, stunServers, internalSTUNMaxConcurrency, shouldStopInternalSTUNSampling, func(reqCtx context.Context, server string) ([]string, time.Duration, error) {
		return discoverFromSTUNServerWithClient(reqCtx, client, server)
	})
}

type stunDiscoverFunc func(ctx context.Context, server string) ([]string, time.Duration, error)

func discoverSTUNWithStrategy(ctx context.Context, stunServers []string, concurrency int, stopFn func(STUNDiscoveryResult) bool, discover stunDiscoverFunc) STUNDiscoveryResult {
	if concurrency <= 0 {
		concurrency = 1
	}

	res := STUNDiscoveryResult{
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
	res.MappedAddrs, dropped = sanitizeMappedAddrs(res.MappedAddrs)
	if len(dropped) > 0 {
		res.Errors = append(res.Errors, fmt.Sprintf("dropped invalid mapped_addrs: %v", dropped))
	}

	if len(res.MappedAddrs) > 4 {
		res.MappedAddrs = res.MappedAddrs[:4]
	}
	return res
}

func shouldStopInternalSTUNSampling(res STUNDiscoveryResult) bool {
	valid, _ := sanitizeMappedAddrs(res.MappedAddrs)
	if len(valid) >= internalSTUNMinMappedAddrs && res.OkCount >= internalSTUNMinMappedAddrs {
		return true
	}

	uniq := make(map[string]struct{}, len(valid))
	for _, addr := range valid {
		uniq[addr] = struct{}{}
	}
	return len(uniq) >= internalSTUNPreferredAddrs
}

func discoverFromSTUNServer(ctx context.Context, conn *net.UDPConn, addr string) ([]string, time.Duration, error) {
	external, other, rtt, err := doSTUNRequest(ctx, conn, addr)
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

	external2, _, _, err := doSTUNRequest(ctx, conn, other)
	if err != nil {
		return out, rtt, nil
	}
	if external2 != "" {
		out = append(out, external2)
	}
	return out, rtt, nil
}

func discoverFromSTUNServerWithClient(ctx context.Context, client *sharedSTUNClient, addr string) ([]string, time.Duration, error) {
	external, other, rtt, err := doSTUNRequestWithClient(ctx, client, addr)
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

	external2, _, _, err := doSTUNRequestWithClient(ctx, client, other)
	if err != nil {
		return out, rtt, nil
	}
	if external2 != "" {
		out = append(out, external2)
	}
	return out, rtt, nil
}

func doSTUNRequest(ctx context.Context, conn *net.UDPConn, addr string) (externalAddr string, otherAddr string, rtt time.Duration, err error) {
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

	start := time.Now()
	if _, err := conn.WriteToUDP(req.Raw, raddr); err != nil {
		return "", "", 0, err
	}

	deadline := time.Now().Add(stunResponseTimeout)
	if dl, ok := ctx.Deadline(); ok && dl.Before(deadline) {
		deadline = dl
	}

	var resp stun.Message
	buf := make([]byte, 2048)
	for {
		_ = conn.SetReadDeadline(deadline)
		n, _, readErr := conn.ReadFromUDP(buf)
		_ = conn.SetReadDeadline(time.Time{})
		if readErr != nil {
			if ctx.Err() != nil {
				return "", "", 0, ctx.Err()
			}
			return "", "", 0, readErr
		}

		resp.Raw = append(resp.Raw[:0], buf[:n]...)
		if err := resp.Decode(); err != nil {
			continue
		}
		if resp.Type.Method != stun.MethodBinding {
			continue
		}
		if !slices.Equal(resp.TransactionID[:], req.TransactionID[:]) {
			continue
		}
		break
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

type sharedSTUNClient struct {
	conn     *net.UDPConn
	stopCh   chan struct{}
	doneCh   chan struct{}
	stopOnce sync.Once

	mu      sync.Mutex
	pending map[string]chan []byte
}

func newSharedSTUNClient(conn *net.UDPConn) *sharedSTUNClient {
	client := &sharedSTUNClient{
		conn:    conn,
		stopCh:  make(chan struct{}),
		doneCh:  make(chan struct{}),
		pending: make(map[string]chan []byte),
	}
	go client.readLoop()
	return client
}

func (c *sharedSTUNClient) Close() {
	c.stopOnce.Do(func() {
		close(c.stopCh)
		_ = c.conn.SetReadDeadline(time.Now())
		<-c.doneCh
	})
}

func (c *sharedSTUNClient) roundTrip(ctx context.Context, raddr *net.UDPAddr, req *stun.Message) (stun.Message, error) {
	key := stunTxKey(req.TransactionID)
	respCh := make(chan []byte, 1)
	if err := c.register(key, respCh); err != nil {
		return stun.Message{}, err
	}
	defer c.unregister(key)

	if _, err := c.conn.WriteToUDP(req.Raw, raddr); err != nil {
		return stun.Message{}, err
	}

	select {
	case raw := <-respCh:
		var msg stun.Message
		msg.Raw = raw
		if err := msg.Decode(); err != nil {
			return stun.Message{}, err
		}
		return msg, nil
	case <-ctx.Done():
		return stun.Message{}, ctx.Err()
	case <-c.stopCh:
		return stun.Message{}, context.Canceled
	}
}

func (c *sharedSTUNClient) register(key string, respCh chan []byte) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	select {
	case <-c.stopCh:
		return context.Canceled
	default:
	}
	c.pending[key] = respCh
	return nil
}

func (c *sharedSTUNClient) unregister(key string) {
	c.mu.Lock()
	delete(c.pending, key)
	c.mu.Unlock()
}

func (c *sharedSTUNClient) readLoop() {
	defer close(c.doneCh)
	defer c.clearPending()
	defer func() { _ = c.conn.SetReadDeadline(time.Time{}) }()

	buf := make([]byte, 2048)
	for {
		select {
		case <-c.stopCh:
			return
		default:
		}

		_ = c.conn.SetReadDeadline(time.Now().Add(stunReadLoopPollInterval))
		n, _, err := c.conn.ReadFromUDP(buf)
		if err != nil {
			var netErr net.Error
			if errors.As(err, &netErr) && netErr.Timeout() {
				continue
			}
			select {
			case <-c.stopCh:
				return
			default:
				continue
			}
		}

		var msg stun.Message
		msg.Raw = append(msg.Raw[:0], buf[:n]...)
		if err := msg.Decode(); err != nil {
			continue
		}
		if msg.Type.Method != stun.MethodBinding {
			continue
		}

		key := stunTxKey(msg.TransactionID)
		c.mu.Lock()
		respCh := c.pending[key]
		c.mu.Unlock()
		if respCh == nil {
			continue
		}

		raw := append([]byte(nil), buf[:n]...)
		select {
		case respCh <- raw:
		default:
		}
	}
}

func (c *sharedSTUNClient) clearPending() {
	c.mu.Lock()
	defer c.mu.Unlock()
	for key := range c.pending {
		delete(c.pending, key)
	}
}

func stunTxKey(txID [stun.TransactionIDSize]byte) string {
	return string(txID[:])
}

func doSTUNRequestWithClient(ctx context.Context, client *sharedSTUNClient, addr string) (externalAddr string, otherAddr string, rtt time.Duration, err error) {
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
		reqCtx, cancel = context.WithTimeout(ctx, stunResponseTimeout)
		defer cancel()
	}

	start := time.Now()
	resp, err := client.roundTrip(reqCtx, raddr, req)
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

func sanitizeMappedAddrs(addrs []string) (valid []string, dropped []string) {
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
