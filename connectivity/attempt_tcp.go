package connectivity

import (
	"context"
	"errors"
	"fmt"
	"math/rand/v2"
	"net"
	"net/netip"
	"strings"
	"sync"
	"time"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/wire"
)

const (
	maxTCPSendRandomPorts   = 128
	maxTCPListenRandomPorts = 32
)

func attemptTCPDirect(ctx context.Context, sid string, key []byte, listener *net.TCPListener, candidates []netip.AddrPort, cfg AttemptConfig, emit func(event.Event), path string) (*AttemptResult, error) {
	_ = sid
	_ = key

	if listener == nil {
		return nil, errors.New("tcp listener is required")
	}

	evStart := "attempt.tcp4.start"
	evOK := "attempt.tcp4.ok"
	evFail := "attempt.tcp4.fail"
	evStartMsg := "attempt tcp4 direct"
	evOKMsg := "tcp4 direct ok"
	evFailMsg := "tcp4 direct failed"
	timeout := cfg.AttemptPortmapTimeout
	if path == "direct_tcp6" {
		evStart = "attempt.tcp6.start"
		evOK = "attempt.tcp6.ok"
		evFail = "attempt.tcp6.fail"
		evStartMsg = "attempt tcp6 direct"
		evOKMsg = "tcp6 direct ok"
		evFailMsg = "tcp6 direct failed"
		timeout = cfg.AttemptV6Timeout
	}

	logutil.Debugf("diagnostic tcp direct start: sid=%s path=%s timeout_ms=%d candidate_count=%d", sid, path, int(timeout.Milliseconds()), len(candidates))
	emit(event.Event{Stage: event.StageAttempt, Kind: event.KindStart, Name: evStart, Msg: evStartMsg})
	for _, cand := range candidates {
		emit(event.Event{
			Stage: event.StageAttempt,
			Kind:  event.KindStart,
			Name:  "attempt.candidate.begin",
			Msg:   "candidate begin",
			KVs: map[string]any{
				"path":      path,
				"candidate": cand.String(),
			},
		})
	}

	stepStart := time.Now()

	subCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	var wg sync.WaitGroup

	resultCh := make(chan TCPConn, 16)

	dialOK := make(map[netip.AddrPort]bool, len(candidates))
	dialErr := make(map[netip.AddrPort]error, len(candidates))
	var dialMu sync.Mutex

	wg.Add(1)
	go func() {
		defer wg.Done()
		for {
			_ = listener.SetDeadline(time.Now().Add(200 * time.Millisecond))
			conn, err := listener.AcceptTCP()
			_ = listener.SetDeadline(time.Time{})
			if err != nil {
				select {
				case <-subCtx.Done():
					return
				default:
				}

				var ne net.Error
				if errors.As(err, &ne) && ne.Timeout() {
					continue
				}
				return
			}

			select {
			case resultCh <- TCPConn{Conn: conn, Origin: TCPConnOriginAccept}:
			default:
				_ = conn.Close()
			}
		}
	}()

	for _, cand := range candidates {
		cand := cand
		wg.Add(1)
		go func() {
			defer wg.Done()
			network := "tcp4"
			if cand.Addr().Is6() {
				network = "tcp6"
			}

			dialer := &net.Dialer{Timeout: timeout}
			c, err := dialer.DialContext(subCtx, network, cand.String())
			if err != nil {
				dialMu.Lock()
				dialErr[cand] = err
				dialMu.Unlock()
				return
			}

			tcpConn, ok := c.(*net.TCPConn)
			if !ok {
				_ = c.Close()
				dialMu.Lock()
				dialErr[cand] = errors.New("dial returned non-tcp conn")
				dialMu.Unlock()
				return
			}

			dialMu.Lock()
			dialOK[cand] = true
			dialMu.Unlock()

			select {
			case resultCh <- TCPConn{Conn: tcpConn, Origin: TCPConnOriginDial}:
			default:
				_ = tcpConn.Close()
			}
		}()
	}

	go func() {
		wg.Wait()
		close(resultCh)
	}()

	settleOnce := sync.Once{}
	startSettle := func() {
		settleOnce.Do(func() {
			time.AfterFunc(200*time.Millisecond, cancel)
		})
	}

	tcpConns := make([]TCPConn, 0, 4)
	for c := range resultCh {
		if c.Conn == nil {
			continue
		}
		if len(tcpConns) < 8 {
			tcpConns = append(tcpConns, c)
			startSettle()
			continue
		}
		_ = c.Conn.Close()
		cancel()
	}

	for _, cand := range candidates {
		dialMu.Lock()
		ok := dialOK[cand]
		err := dialErr[cand]
		dialMu.Unlock()

		if ok {
			emit(event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindOK,
				Name:  "attempt.candidate.end",
				Msg:   "candidate ok",
				KVs: map[string]any{
					"path":      path,
					"candidate": cand.String(),
					"reason":    "reachable",
				},
			})
			continue
		}

		reason := "timeout"
		if err != nil {
			reason = "dial_error"
		}
		emit(event.Event{
			Stage: event.StageAttempt,
			Kind:  event.KindInfo,
			Name:  "attempt.candidate.end",
			Msg:   "candidate failed",
			KVs: map[string]any{
				"path":      path,
				"candidate": cand.String(),
				"reason":    reason,
			},
		})
	}

	if len(tcpConns) > 0 {
		logutil.Debugf("diagnostic tcp direct ok: sid=%s path=%s conns=%d elapsed_ms=%d", sid, path, len(tcpConns), time.Since(stepStart).Milliseconds())
		emit(event.Event{
			Stage: event.StageAttempt,
			Kind:  event.KindOK,
			Name:  evOK,
			Msg:   evOKMsg,
			KVs: map[string]any{
				"conns": len(tcpConns),
				"ms":    time.Since(stepStart).Milliseconds(),
			},
		})
		return &AttemptResult{Path: path, TCPConns: tcpConns}, nil
	}

	err := subCtx.Err()
	if err == nil {
		err = errors.New("no tcp connections established")
	}
	logutil.Debugf("diagnostic tcp direct failed: sid=%s path=%s elapsed_ms=%d err=%v", sid, path, time.Since(stepStart).Milliseconds(), err)
	emit(event.Event{Stage: event.StageAttempt, Kind: event.KindInfo, Name: evFail, Msg: evFailMsg, Err: err.Error()})
	return nil, err
}

func attemptTCPPunching(ctx context.Context, sid string, key []byte, baseListener *net.TCPListener, resp *wire.NatHoleResp, cfg AttemptConfig, emit func(event.Event)) (*AttemptResult, error) {
	_ = sid
	_ = key

	if baseListener == nil {
		return nil, errors.New("tcp4 listener is required for tcp punching")
	}
	if resp == nil {
		return nil, errors.New("nil NatHoleResp")
	}

	if !resp.TCPPunchingEnabled {
		if cfg.P2PNetwork == P2PNetworkTCPOnly {
			return nil, fmt.Errorf("tcp punching disabled: %s", stringsOr(resp.TCPPunchingError, "unknown"))
		}
		emit(event.Event{
			Stage: event.StageAttempt,
			Kind:  event.KindInfo,
			Name:  "attempt.tcp_punching.skip",
			Msg:   "tcp punching skipped",
			KVs: map[string]any{
				"reason": stringsOr(resp.TCPPunchingError, "disabled"),
			},
		})
		return nil, nil
	}
	if len(resp.TCPCandidateAddrs) == 0 && len(resp.TCPAssistedAddrs) == 0 {
		if cfg.P2PNetwork == P2PNetworkTCPOnly {
			return nil, errors.New("tcp punching enabled but tcp candidate targets empty")
		}
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindInfo, Name: "attempt.tcp_punching.skip", Msg: "tcp punching skipped", Err: "tcp candidate targets empty"})
		return nil, nil
	}
	if resp.TCPDetectBehavior == nil {
		if cfg.P2PNetwork == P2PNetworkTCPOnly {
			return nil, errors.New("tcp punching enabled but tcp_detect_behavior missing")
		}
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindInfo, Name: "attempt.tcp_punching.skip", Msg: "tcp punching skipped", Err: "tcp_detect_behavior missing"})
		return nil, nil
	}

	maxConcurrency := 64
	totalBudget := 5 * time.Second
	dialTimeout := 1500 * time.Millisecond
	dialRoundInterval := 200 * time.Millisecond
	if cfg.P2PNetwork == P2PNetworkTCPOnly {
		totalBudget = 10 * time.Second
		dialTimeout = 2500 * time.Millisecond
	}
	settleWindow := 200 * time.Millisecond
	sendRandomPorts := effectiveTCPSendRandomPorts(resp.TCPDetectBehavior.SendRandomPorts)
	listenRandomPorts := effectiveTCPListenRandomPorts(resp.TCPDetectBehavior.ListenRandomPorts)
	targets, err := buildTCPPunchTargets(resp.TCPCandidateAddrs, resp.TCPAssistedAddrs, resp.TCPDetectBehavior.CandidatePorts, sendRandomPorts)
	if err != nil {
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindFail, Name: "attempt.tcp_punching.fail", Msg: "tcp punching target build failed", Err: err.Error()})
		return nil, err
	}
	logutil.Debugf(
		"diagnostic tcp punching targets: sid=%s candidate_addrs=%v assisted_addrs=%v candidate_ports=%v assisted_exact=%d candidate_exact=%d candidate_expanded=%d target_count=%d targets=%v",
		sid,
		resp.TCPCandidateAddrs,
		resp.TCPAssistedAddrs,
		resp.TCPDetectBehavior.CandidatePorts,
		targets.AssistedExactCount,
		targets.CandidateExactCount,
		targets.CandidateExpandedCount,
		len(targets.Targets),
		targets.Targets,
	)

	emit(event.Event{
		Stage: event.StageAttempt,
		Kind:  event.KindStart,
		Name:  "attempt.tcp_punching.start",
		Msg:   "attempt tcp punching",
		KVs: map[string]any{
			"mode":                          resp.TCPDetectBehavior.Mode,
			"role":                          resp.TCPDetectBehavior.Role,
			"send_delay_ms":                 resp.TCPDetectBehavior.SendDelayMs,
			"read_timeout_ms":               resp.TCPDetectBehavior.ReadTimeoutMs,
			"candidate_addrs":               len(resp.TCPCandidateAddrs),
			"assisted_addrs":                len(resp.TCPAssistedAddrs),
			"assisted_exact_targets":        targets.AssistedExactCount,
			"candidate_exact_targets":       targets.CandidateExactCount,
			"candidate_expanded_targets":    targets.CandidateExpandedCount,
			"targets":                       len(targets.Targets),
			"candidate_ports":               len(resp.TCPDetectBehavior.CandidatePorts),
			"send_random_ports_requested":   resp.TCPDetectBehavior.SendRandomPorts,
			"send_random_ports":             sendRandomPorts,
			"listen_random_ports_requested": resp.TCPDetectBehavior.ListenRandomPorts,
			"listen_random_ports":           listenRandomPorts,
			"max_concurrency":               maxConcurrency,
			"total_budget_ms":               int(totalBudget.Milliseconds()),
			"dial_timeout_ms":               int(dialTimeout.Milliseconds()),
			"dial_round_interval_ms":        int(dialRoundInterval.Milliseconds()),
			"settle_window_ms":              int(settleWindow.Milliseconds()),
		},
	})

	stepStart := time.Now()

	subCtx, cancel := context.WithTimeout(ctx, totalBudget)
	defer cancel()

	listeners := []*net.TCPListener{baseListener}
	extraListeners := make([]*net.TCPListener, 0, listenRandomPorts)
	defer func() {
		for _, ln := range extraListeners {
			_ = ln.Close()
		}
	}()

	for range listenRandomPorts {
		ln, err := listenTCPWithReuseAddr("tcp4", "0.0.0.0:0")
		if err != nil {
			emit(event.Event{Stage: event.StageAttempt, Kind: event.KindInfo, Name: "attempt.tcp_punching.listen_random.failed", Msg: "listen random tcp port failed", Err: err.Error()})
			continue
		}
		extraListeners = append(extraListeners, ln)
		listeners = append(listeners, ln)
	}

	type dialJob struct {
		srcPort int
		dst     netip.AddrPort
	}

	dialJobs := make([]dialJob, 0, len(listeners)*len(targets.Targets))
	for _, ln := range listeners {
		addr, ok := ln.Addr().(*net.TCPAddr)
		if !ok {
			continue
		}
		for _, dst := range targets.Targets {
			dialJobs = append(dialJobs, dialJob{srcPort: addr.Port, dst: dst})
		}
	}

	jobCh := make(chan dialJob)
	resultCh := make(chan TCPConn, 32)

	var wg sync.WaitGroup

	for _, ln := range listeners {
		ln := ln
		wg.Add(1)
		go func() {
			defer wg.Done()
			for {
				_ = ln.SetDeadline(time.Now().Add(200 * time.Millisecond))
				conn, err := ln.AcceptTCP()
				_ = ln.SetDeadline(time.Time{})
				if err != nil {
					select {
					case <-subCtx.Done():
						return
					default:
					}

					var ne net.Error
					if errors.As(err, &ne) && ne.Timeout() {
						continue
					}
					return
				}

				select {
				case resultCh <- TCPConn{Conn: conn, Origin: TCPConnOriginAccept}:
				default:
					_ = conn.Close()
				}
			}
		}()
	}

	for i := 0; i < maxConcurrency; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for job := range jobCh {
				select {
				case <-subCtx.Done():
					return
				default:
				}

				localAddr := &net.TCPAddr{IP: net.IPv4zero, Port: job.srcPort}
				dialer := newTCPDialerWithReuseAddr(localAddr, dialTimeout)
				c, err := dialer.DialContext(subCtx, "tcp4", job.dst.String())
				if err != nil {
					continue
				}

				conn, ok := c.(*net.TCPConn)
				if !ok {
					_ = c.Close()
					continue
				}

				select {
				case resultCh <- TCPConn{Conn: conn, Origin: TCPConnOriginDial}:
				default:
					_ = conn.Close()
				}
			}
		}()
	}

	emit(event.Event{
		Stage: event.StageAttempt,
		Kind:  event.KindInfo,
		Name:  "attempt.tcp_punching.plan",
		Msg:   "tcp punching plan",
		KVs: map[string]any{
			"listen_ports":        len(listeners),
			"send_random_ports":   sendRandomPorts,
			"listen_random_ports": listenRandomPorts,
			"targets":             len(targets.Targets),
		},
	})

	emit(event.Event{
		Stage: event.StageAttempt,
		Kind:  event.KindInfo,
		Name:  "attempt.tcp_punching.recv.start",
		Msg:   "tcp punching receive loop start",
		KVs: map[string]any{
			"listen_ports":        len(listeners),
			"listen_random_ports": listenRandomPorts,
		},
	})

	probeStartOnce := sync.Once{}
	probeFirstOnce := sync.Once{}
	connFirstOnce := sync.Once{}

	go func() {
		defer close(jobCh)

		probeStartOnce.Do(func() {
			emit(event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindInfo,
				Name:  "attempt.tcp_punching.probe.start",
				Msg:   "tcp punching probe loop start",
				KVs: map[string]any{
					"send_delay_ms":          resp.TCPDetectBehavior.SendDelayMs,
					"total_budget_ms":        int(totalBudget.Milliseconds()),
					"dial_round_interval_ms": int(dialRoundInterval.Milliseconds()),
				},
			})
		})

		delay := time.Duration(resp.TCPDetectBehavior.SendDelayMs) * time.Millisecond
		if delay > 0 {
			timer := time.NewTimer(delay)
			defer timer.Stop()
			select {
			case <-timer.C:
			case <-subCtx.Done():
				return
			}
		}

		probeFirstOnce.Do(func() {
			emit(event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindInfo,
				Name:  "attempt.tcp_punching.probe.first",
				Msg:   "tcp punching first probe burst",
			})
		})

		sendRound := func() bool {
			for _, job := range dialJobs {
				select {
				case <-subCtx.Done():
					return false
				case jobCh <- job:
				}
			}
			return true
		}
		if !sendRound() {
			return
		}

		ticker := time.NewTicker(dialRoundInterval)
		defer ticker.Stop()
		for {
			select {
			case <-subCtx.Done():
				return
			case <-ticker.C:
			}
			if !sendRound() {
				return
			}
		}
	}()

	go func() {
		wg.Wait()
		close(resultCh)
	}()

	settleOnce := sync.Once{}
	startSettle := func() {
		settleOnce.Do(func() {
			time.AfterFunc(settleWindow, func() {
				emit(event.Event{
					Stage: event.StageAttempt,
					Kind:  event.KindInfo,
					Name:  "attempt.tcp_punching.canceled",
					Msg:   "tcp punching canceled after settle window",
					KVs: map[string]any{
						"reason": "winner_selected",
					},
				})
				cancel()
			})
		})
	}

	tcpConns := make([]TCPConn, 0, 8)
	earlyStop := false

	for c := range resultCh {
		if c.Conn == nil {
			continue
		}
		connFirstOnce.Do(func() {
			winnerTargetSource := "unknown_accept"
			if c.Origin == TCPConnOriginDial {
				if remote, err := netip.ParseAddrPort(c.Conn.RemoteAddr().String()); err == nil {
					winnerTargetSource = targets.Source(remote)
				} else {
					winnerTargetSource = "unknown_dial"
				}
			}
			emit(event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindInfo,
				Name:  "attempt.tcp_punching.conn.first",
				Msg:   "tcp punching first connection observed",
				KVs: map[string]any{
					"origin": c.Origin,
					"laddr":  c.Conn.LocalAddr().String(),
					"raddr":  c.Conn.RemoteAddr().String(),
				},
			})
			emit(event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindOK,
				Name:  "attempt.tcp_punching.winner",
				Msg:   "tcp punching winner selected",
				KVs: map[string]any{
					"origin":               c.Origin,
					"elapsed_ms":           time.Since(stepStart).Milliseconds(),
					"winner_target_source": winnerTargetSource,
				},
			})
			logutil.Debugf(
				"diagnostic tcp punching winner: sid=%s origin=%s elapsed_ms=%d winner_target_source=%s laddr=%s raddr=%s",
				sid,
				c.Origin,
				time.Since(stepStart).Milliseconds(),
				winnerTargetSource,
				c.Conn.LocalAddr().String(),
				c.Conn.RemoteAddr().String(),
			)
		})
		if len(tcpConns) < 8 {
			tcpConns = append(tcpConns, c)
			earlyStop = true
			startSettle()
			continue
		}
		_ = c.Conn.Close()
		cancel()
	}

	if len(tcpConns) == 0 {
		err := subCtx.Err()
		if err == nil {
			err = errors.New("tcp punching failed: no connections established")
		}
		emit(event.Event{
			Stage: event.StageAttempt,
			Kind:  event.KindInfo,
			Name:  "attempt.tcp_punching.timeout",
			Msg:   "tcp punching timeout",
			Err:   err.Error(),
			KVs: map[string]any{
				"elapsed_ms": time.Since(stepStart).Milliseconds(),
			},
		})
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindFail, Name: "attempt.tcp_punching.fail", Msg: "tcp punching failed", Err: err.Error()})
		return nil, err
	}

	emit(event.Event{
		Stage: event.StageAttempt,
		Kind:  event.KindOK,
		Name:  "attempt.tcp_punching.ok",
		Msg:   "tcp punching ok",
		KVs: map[string]any{
			"conns":      len(tcpConns),
			"successes":  len(tcpConns),
			"early_stop": earlyStop,
			"ms":         time.Since(stepStart).Milliseconds(),
		},
	})
	return &AttemptResult{Path: "punching_tcp4", TCPConns: tcpConns}, nil
}

type tcpPunchTargets struct {
	Targets []netip.AddrPort

	AssistedExactCount     int
	CandidateExactCount    int
	CandidateExpandedCount int

	assistedExact     map[netip.AddrPort]struct{}
	candidateExact    map[netip.AddrPort]struct{}
	candidateExpanded map[netip.AddrPort]struct{}
}

func (t tcpPunchTargets) Source(dst netip.AddrPort) string {
	if _, ok := t.assistedExact[dst]; ok {
		return "assisted_exact"
	}
	if _, ok := t.candidateExact[dst]; ok {
		return "candidate_exact"
	}
	if _, ok := t.candidateExpanded[dst]; ok {
		return "candidate_expanded"
	}
	return "unknown"
}

func buildTCPPunchTargets(candidateAddrs []string, assistedAddrs []string, candidatePorts []wire.PortsRange, sendRandomPorts int) (tcpPunchTargets, error) {
	parsedCandidates := ParseDirectAddrPorts(candidateAddrs)
	if len(parsedCandidates.Invalid) > 0 {
		return tcpPunchTargets{}, fmt.Errorf("invalid tcp_candidate_addrs: %v", parsedCandidates.Invalid)
	}
	parsedAssisted := ParseDirectAddrPorts(assistedAddrs)
	if len(parsedAssisted.Invalid) > 0 {
		return tcpPunchTargets{}, fmt.Errorf("invalid tcp_assisted_addrs: %v", parsedAssisted.Invalid)
	}

	assistedExact := make([]netip.AddrPort, 0, len(parsedAssisted.Addrs))
	for _, ap := range parsedAssisted.Addrs {
		if ap.Addr().Is4() {
			assistedExact = append(assistedExact, ap)
		}
	}
	candidateExact := make([]netip.AddrPort, 0, len(parsedCandidates.Addrs))
	for _, ap := range parsedCandidates.Addrs {
		if ap.Addr().Is4() {
			candidateExact = append(candidateExact, ap)
		}
	}

	assistedExactSet := make(map[netip.AddrPort]struct{}, len(assistedExact))
	assistedExactDedup := make([]netip.AddrPort, 0, len(assistedExact))
	for _, ap := range assistedExact {
		if _, ok := assistedExactSet[ap]; ok {
			continue
		}
		assistedExactSet[ap] = struct{}{}
		assistedExactDedup = append(assistedExactDedup, ap)
	}

	candidateExactSet := make(map[netip.AddrPort]struct{}, len(candidateExact))
	candidateExactDedup := make([]netip.AddrPort, 0, len(candidateExact))
	candidateIPs := make([]netip.Addr, 0, len(candidateExact))
	seenIP := make(map[netip.Addr]struct{}, len(candidateExact))
	for _, ap := range candidateExact {
		if _, ok := candidateExactSet[ap]; ok {
			continue
		}
		candidateExactSet[ap] = struct{}{}
		candidateExactDedup = append(candidateExactDedup, ap)
		if _, ok := seenIP[ap.Addr()]; ok {
			continue
		}
		seenIP[ap.Addr()] = struct{}{}
		candidateIPs = append(candidateIPs, ap.Addr())
	}

	expanded := make([]netip.AddrPort, 0, len(candidatePorts)*len(candidateIPs))
	for _, pr := range candidatePorts {
		for p := pr.From; p <= pr.To; p++ {
			if p <= 0 || p > 65535 {
				continue
			}
			for _, ip := range candidateIPs {
				expanded = append(expanded, netip.AddrPortFrom(ip, uint16(p)))
			}
		}
	}

	if sendRandomPorts > 0 {
		used := make(map[int]struct{}, sendRandomPorts)
		getPort := func() int {
			for range 10 {
				port := rand.IntN(65535-1024) + 1024
				if _, ok := used[port]; ok {
					continue
				}
				used[port] = struct{}{}
				return port
			}
			return 0
		}
		for range sendRandomPorts {
			port := getPort()
			if port == 0 {
				continue
			}
			for _, ip := range candidateIPs {
				expanded = append(expanded, netip.AddrPortFrom(ip, uint16(port)))
			}
		}
	}

	expectedTargets := len(assistedExactDedup) + len(candidateExactDedup) + len(expanded)
	combined := make([]netip.AddrPort, 0, expectedTargets)
	seenCombined := make(map[netip.AddrPort]struct{}, expectedTargets)
	add := func(ap netip.AddrPort) bool {
		if _, ok := seenCombined[ap]; ok {
			return false
		}
		seenCombined[ap] = struct{}{}
		combined = append(combined, ap)
		return true
	}

	for _, ap := range assistedExactDedup {
		add(ap)
	}
	candidateExactCount := 0
	for _, ap := range candidateExactDedup {
		if add(ap) {
			candidateExactCount++
		}
	}

	candidateExpandedSet := make(map[netip.AddrPort]struct{}, len(expanded))
	exactCount := len(combined)
	for _, ap := range expanded {
		if _, ok := assistedExactSet[ap]; ok {
			continue
		}
		if _, ok := candidateExactSet[ap]; ok {
			continue
		}
		if add(ap) {
			candidateExpandedSet[ap] = struct{}{}
		}
	}

	return tcpPunchTargets{
		Targets:                combined,
		AssistedExactCount:     len(assistedExactDedup),
		CandidateExactCount:    candidateExactCount,
		CandidateExpandedCount: len(combined) - exactCount,
		assistedExact:          assistedExactSet,
		candidateExact:         candidateExactSet,
		candidateExpanded:      candidateExpandedSet,
	}, nil
}

func effectiveTCPSendRandomPorts(requested int) int {
	return clampTCPRandomPorts(requested, maxTCPSendRandomPorts)
}

func effectiveTCPListenRandomPorts(requested int) int {
	return clampTCPRandomPorts(requested, maxTCPListenRandomPorts)
}

func clampTCPRandomPorts(requested int, limit int) int {
	if requested <= 0 {
		return 0
	}
	if requested > limit {
		return limit
	}
	return requested
}

func stringsOr(value string, fallback string) string {
	if strings.TrimSpace(value) == "" {
		return fallback
	}
	return strings.TrimSpace(value)
}
