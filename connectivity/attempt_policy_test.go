package connectivity

import (
	"bytes"
	"context"
	"encoding/json"
	"net"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/wire"
)

func parseEventNames(t *testing.T, buf *bytes.Buffer) []string {
	t.Helper()

	events := parseEvents(t, buf)
	out := make([]string, 0, len(events))
	for _, ev := range events {
		out = append(out, ev.Name)
	}
	return out
}

func parseEvents(t *testing.T, buf *bytes.Buffer) []event.Event {
	t.Helper()

	lines := strings.Split(strings.TrimSpace(buf.String()), "\n")
	out := make([]event.Event, 0, len(lines))
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		var ev event.Event
		if err := json.Unmarshal([]byte(line), &ev); err != nil {
			t.Fatalf("unmarshal event: %v (line=%q)", err, line)
		}
		out = append(out, ev)
	}
	return out
}

func indexOf(names []string, want string) int {
	for i, n := range names {
		if n == want {
			return i
		}
	}
	return -1
}

func TestAttempt_AutoAttemptsTCPBeforeUDP(t *testing.T) {
	a, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen A: %v", err)
	}
	defer a.Close()

	b, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen B: %v", err)
	}
	defer b.Close()

	lnA, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listenTCP A: %v", err)
	}
	defer lnA.Close()

	lnB, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listenTCP B: %v", err)
	}
	defer lnB.Close()

	sid := "sid-auto-order"
	key := []byte("0123456789abcdef")

	respA := &wire.NatHoleResp{
		PeerDirectAddrs:    []string{b.LocalAddr().String()},
		PeerTCPDirectAddrs: []string{"127.0.0.1:1"},
	}
	respB := &wire.NatHoleResp{
		PeerDirectAddrs:    []string{a.LocalAddr().String()},
		PeerTCPDirectAddrs: []string{"127.0.0.1:1"},
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	var log bytes.Buffer
	em := event.NewEmitter(&log, "test")

	cfgA := AttemptConfig{
		P2PNetwork:            P2PNetworkAuto,
		AttemptPortmapTimeout: 120 * time.Millisecond,
		DirectSendCount:       2,
		DirectSendInterval:    20 * time.Millisecond,
		Emitter:               em,
	}
	cfgB := AttemptConfig{
		P2PNetwork:            P2PNetworkAuto,
		AttemptPortmapTimeout: 120 * time.Millisecond,
		DirectSendCount:       2,
		DirectSendInterval:    20 * time.Millisecond,
	}

	type out struct {
		res *AttemptResult
		err error
	}
	ch := make(chan out, 2)
	go func() {
		res, err := Attempt(ctx, sid, key, a, nil, lnA, nil, respA, cfgA)
		ch <- out{res: res, err: err}
	}()
	go func() {
		res, err := Attempt(ctx, sid, key, b, nil, lnB, nil, respB, cfgB)
		ch <- out{res: res, err: err}
	}()

	o1 := <-ch
	o2 := <-ch
	if o1.err != nil || o2.err != nil {
		t.Fatalf("attempt errors: o1=%v o2=%v", o1.err, o2.err)
	}
	if o1.res.Path != "direct_ipv4" || o2.res.Path != "direct_ipv4" {
		t.Fatalf("unexpected paths: o1=%v o2=%v", o1.res.Path, o2.res.Path)
	}

	names := parseEventNames(t, &log)
	iTCP := indexOf(names, "attempt.tcp4.start")
	iUDP := indexOf(names, "attempt.v4.start")
	if iTCP < 0 || iUDP < 0 || iTCP > iUDP {
		t.Fatalf("unexpected attempt order: tcp4.start=%d udp4.start=%d names=%v", iTCP, iUDP, names)
	}
}

func TestAttempt_UDPOnlySkipsTCP(t *testing.T) {
	a, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen A: %v", err)
	}
	defer a.Close()

	b, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listen B: %v", err)
	}
	defer b.Close()

	lnA, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listenTCP A: %v", err)
	}
	defer lnA.Close()

	lnB, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listenTCP B: %v", err)
	}
	defer lnB.Close()

	sid := "sid-udp-only"
	key := []byte("0123456789abcdef")

	respA := &wire.NatHoleResp{
		PeerDirectAddrs:    []string{b.LocalAddr().String()},
		PeerTCPDirectAddrs: []string{lnB.Addr().String()},
	}
	respB := &wire.NatHoleResp{
		PeerDirectAddrs:    []string{a.LocalAddr().String()},
		PeerTCPDirectAddrs: []string{lnA.Addr().String()},
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	var log bytes.Buffer
	em := event.NewEmitter(&log, "test")

	cfgA := AttemptConfig{
		P2PNetwork:         P2PNetworkUDPOnly,
		DirectSendCount:    2,
		DirectSendInterval: 20 * time.Millisecond,
		Emitter:            em,
	}

	type out struct {
		res *AttemptResult
		err error
	}
	ch := make(chan out, 2)
	go func() {
		res, err := Attempt(ctx, sid, key, a, nil, lnA, nil, respA, cfgA)
		ch <- out{res: res, err: err}
	}()
	go func() {
		res, err := Attempt(ctx, sid, key, b, nil, lnB, nil, respB, AttemptConfig{P2PNetwork: P2PNetworkUDPOnly})
		ch <- out{res: res, err: err}
	}()

	o1 := <-ch
	o2 := <-ch
	if o1.err != nil || o2.err != nil {
		t.Fatalf("attempt errors: o1=%v o2=%v", o1.err, o2.err)
	}
	if o1.res.Path != "direct_ipv4" || o2.res.Path != "direct_ipv4" {
		t.Fatalf("unexpected paths: o1=%v o2=%v", o1.res.Path, o2.res.Path)
	}

	names := parseEventNames(t, &log)
	if indexOf(names, "attempt.tcp4.start") >= 0 || indexOf(names, "attempt.tcp_punching.start") >= 0 {
		t.Fatalf("unexpected tcp attempt in udp_only: names=%v", names)
	}
}

func TestAttempt_TCPOnlySkipsUDP(t *testing.T) {
	lnA, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listenTCP A: %v", err)
	}
	defer lnA.Close()

	lnB, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("listenTCP B: %v", err)
	}
	defer lnB.Close()

	sid := "sid-tcp-only"
	key := []byte("0123456789abcdef")

	respA := &wire.NatHoleResp{PeerTCPDirectAddrs: []string{lnB.Addr().String()}}
	respB := &wire.NatHoleResp{PeerTCPDirectAddrs: []string{lnA.Addr().String()}}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	var log bytes.Buffer
	em := event.NewEmitter(&log, "test")

	cfgA := AttemptConfig{
		P2PNetwork:            P2PNetworkTCPOnly,
		AttemptPortmapTimeout: 800 * time.Millisecond,
		Emitter:               em,
	}

	type out struct {
		res *AttemptResult
		err error
	}
	ch := make(chan out, 2)
	go func() {
		res, err := Attempt(ctx, sid, key, nil, nil, lnA, nil, respA, cfgA)
		ch <- out{res: res, err: err}
	}()
	go func() {
		res, err := Attempt(ctx, sid, key, nil, nil, lnB, nil, respB, AttemptConfig{P2PNetwork: P2PNetworkTCPOnly})
		ch <- out{res: res, err: err}
	}()

	o1 := <-ch
	o2 := <-ch
	if o1.err != nil || o2.err != nil {
		t.Fatalf("attempt errors: o1=%v o2=%v", o1.err, o2.err)
	}
	if o1.res.Path != "direct_tcp4" || o2.res.Path != "direct_tcp4" {
		t.Fatalf("unexpected paths: o1=%v o2=%v", o1.res.Path, o2.res.Path)
	}
	if len(o1.res.TCPConns) == 0 || len(o2.res.TCPConns) == 0 {
		t.Fatalf("expected tcp conns: o1=%d o2=%d", len(o1.res.TCPConns), len(o2.res.TCPConns))
	}

	names := parseEventNames(t, &log)
	if indexOf(names, "attempt.v4.start") >= 0 || indexOf(names, "attempt.punching.start") >= 0 {
		t.Fatalf("unexpected udp attempt in tcp_only: names=%v", names)
	}
}

func TestAttemptTCPPunching_DisabledDoesNotInferFromCandidates(t *testing.T) {
	ln, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("ListenTCP: %v", err)
	}
	defer ln.Close()

	const reason = "tcp punching disabled: insufficient tcp_mapped_addrs samples"
	resp := &wire.NatHoleResp{
		TCPPunchingEnabled: false,
		TCPPunchingError:   reason,
		TCPCandidateAddrs:  []string{"203.0.113.10:5100"},
		TCPDetectBehavior: &wire.TcpDetectBehavior{
			Mode:              2,
			SendRandomPorts:   128,
			ListenRandomPorts: 32,
		},
	}

	var log bytes.Buffer
	em := event.NewEmitter(&log, "test")
	emit := func(ev event.Event) {
		em.Emit(ev)
	}

	res, err := attemptTCPPunching(context.Background(), "sid", nil, ln, resp, AttemptConfig{P2PNetwork: P2PNetworkAuto}, emit)
	if err != nil {
		t.Fatalf("attemptTCPPunching(auto) error = %v, want nil", err)
	}
	if res != nil {
		t.Fatalf("attemptTCPPunching(auto) result = %#v, want nil", res)
	}
	if resp.TCPPunchingEnabled {
		t.Fatalf("attemptTCPPunching(auto) mutated tcp_punching_enabled = true, want false")
	}
	if resp.TCPPunchingError != reason {
		t.Fatalf("attemptTCPPunching(auto) tcp_punching_error = %q, want %q", resp.TCPPunchingError, reason)
	}

	names := parseEventNames(t, &log)
	if indexOf(names, "attempt.tcp_punching.skip") < 0 {
		t.Fatalf("attemptTCPPunching(auto) events = %v, want skip event", names)
	}
	if indexOf(names, "attempt.tcp_punching.start") >= 0 {
		t.Fatalf("attemptTCPPunching(auto) events = %v, want no start event", names)
	}
}

func TestAttemptTCPPunching_DisabledFailsInTCPOnly(t *testing.T) {
	ln, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("ListenTCP: %v", err)
	}
	defer ln.Close()

	const reason = "tcp punching disabled: insufficient tcp_mapped_addrs samples"
	resp := &wire.NatHoleResp{
		TCPPunchingEnabled: false,
		TCPPunchingError:   reason,
		TCPCandidateAddrs:  []string{"203.0.113.10:5100"},
		TCPDetectBehavior: &wire.TcpDetectBehavior{
			Mode:              2,
			SendRandomPorts:   128,
			ListenRandomPorts: 32,
		},
	}

	var log bytes.Buffer
	em := event.NewEmitter(&log, "test")
	emit := func(ev event.Event) {
		em.Emit(ev)
	}

	res, err := attemptTCPPunching(context.Background(), "sid", nil, ln, resp, AttemptConfig{P2PNetwork: P2PNetworkTCPOnly}, emit)
	if err == nil {
		t.Fatalf("attemptTCPPunching(tcp_only) error = nil, want disabled error")
	}
	if !strings.Contains(err.Error(), reason) {
		t.Fatalf("attemptTCPPunching(tcp_only) error = %v, want reason %q", err, reason)
	}
	if res != nil {
		t.Fatalf("attemptTCPPunching(tcp_only) result = %#v, want nil", res)
	}
	if resp.TCPPunchingEnabled {
		t.Fatalf("attemptTCPPunching(tcp_only) mutated tcp_punching_enabled = true, want false")
	}

	names := parseEventNames(t, &log)
	if indexOf(names, "attempt.tcp_punching.start") >= 0 {
		t.Fatalf("attemptTCPPunching(tcp_only) events = %v, want no start event", names)
	}
}

func TestAttemptTCPPunching_InvalidTargetsReturnsBeforeDialing(t *testing.T) {
	ln, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("ListenTCP: %v", err)
	}
	defer ln.Close()

	resp := &wire.NatHoleResp{
		TCPPunchingEnabled: true,
		TCPCandidateAddrs:  []string{"not-an-addr"},
		TCPDetectBehavior: &wire.TcpDetectBehavior{
			Mode: 2,
		},
	}

	var log bytes.Buffer
	em := event.NewEmitter(&log, "test")
	emit := func(ev event.Event) {
		em.Emit(ev)
	}

	res, err := attemptTCPPunching(context.Background(), "sid", nil, ln, resp, AttemptConfig{P2PNetwork: P2PNetworkTCPOnly}, emit)
	if err == nil {
		t.Fatalf("attemptTCPPunching(invalid targets) error = nil, want error")
	}
	if res != nil {
		t.Fatalf("attemptTCPPunching(invalid targets) result = %#v, want nil", res)
	}

	names := parseEventNames(t, &log)
	if indexOf(names, "attempt.tcp_punching.fail") < 0 {
		t.Fatalf("attemptTCPPunching(invalid targets) events = %v, want fail event", names)
	}
}

func TestTCPPunchingRandomPortGuardrails(t *testing.T) {
	if got := effectiveTCPSendRandomPorts(10000); got != maxTCPSendRandomPorts {
		t.Fatalf("effectiveTCPSendRandomPorts(10000) = %d, want %d", got, maxTCPSendRandomPorts)
	}
	if got := effectiveTCPListenRandomPorts(10000); got != maxTCPListenRandomPorts {
		t.Fatalf("effectiveTCPListenRandomPorts(10000) = %d, want %d", got, maxTCPListenRandomPorts)
	}
	if got := effectiveTCPSendRandomPorts(-1); got != 0 {
		t.Fatalf("effectiveTCPSendRandomPorts(-1) = %d, want 0", got)
	}
	if got := effectiveTCPListenRandomPorts(-1); got != 0 {
		t.Fatalf("effectiveTCPListenRandomPorts(-1) = %d, want 0", got)
	}

	targets, err := buildTCPPunchTargets([]string{"203.0.113.10:5100"}, nil, 10000)
	if err != nil {
		t.Fatalf("buildTCPPunchTargets() error = %v, want nil", err)
	}
	if len(targets) > 1+maxTCPSendRandomPorts {
		t.Fatalf("buildTCPPunchTargets() targets = %d, want at most %d", len(targets), 1+maxTCPSendRandomPorts)
	}
}
