package connectivity

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"strconv"
	"sync"
	"time"

	"github.com/miopunch/miopunch/xtcp/nathole"
	"github.com/miopunch/miopunch/xtcp/obs"
)

type GatherConfig struct {
	ListenPort int

	DisableAssistedAddrs bool
	DisablePortMap       bool

	StunServers []string
	StunTimeout time.Duration

	GatherTimeout time.Duration
	SessionLease  time.Duration

	Emitter *obs.Emitter
}

type GatherResult struct {
	UDP4Conn *net.UDPConn
	UDP6Conn *net.UDPConn

	DirectAddrs   []string
	MappedAddrs   []string
	AssistedAddrs []string
}

func Gather(ctx context.Context, sid string, cfg GatherConfig) (*GatherResult, error) {
	if cfg.StunTimeout == 0 {
		cfg.StunTimeout = 3 * time.Second
	}
	if cfg.GatherTimeout == 0 {
		cfg.GatherTimeout = 1500 * time.Millisecond
	}
	if cfg.SessionLease == 0 {
		cfg.SessionLease = 5 * time.Minute
	}

	emit := func(ev obs.Event) {
		if cfg.Emitter != nil {
			ev.SID = sid
			cfg.Emitter.Emit(ev)
		}
	}

	udp4Conn, udp4Port, err := bindUDP4(cfg.ListenPort)
	if err != nil {
		return nil, err
	}

	udp6Conn, udp6Port, udp6Err := bindUDP6(cfg.ListenPort, udp4Port)
	if udp6Err != nil {
		emit(obs.Event{
			Stage: obs.StageGather,
			Kind:  obs.KindInfo,
			Name:  "gather.udp6.unavailable",
			Msg:   "udp6 bind failed",
			Err:   udp6Err.Error(),
		})
	}

	emit(obs.Event{
		Stage: obs.StageGather,
		Kind:  obs.KindStart,
		Name:  "gather.start",
		Msg:   "gather start",
		KVs: map[string]any{
			"udp4_port": udp4Port,
			"udp6_port": udp6Port,
		},
	})

	gatherStart := time.Now()

	// 1) IPv6 local candidates (non-blocking).
	var directCandidates []netip.AddrPort
	if udp6Conn != nil {
		v6Addrs, err := GatherLocalIPv6Candidates()
		if err != nil {
			emit(obs.Event{
				Stage: obs.StageGather,
				Kind:  obs.KindFail,
				Name:  "gather.v6.error",
				Msg:   "ipv6 gather failed",
				Err:   err.Error(),
			})
		} else {
			for _, addr := range v6Addrs {
				directCandidates = append(directCandidates, netip.AddrPortFrom(addr, uint16(udp6Port)))
			}
			emit(obs.Event{
				Stage: obs.StageGather,
				Kind:  obs.KindOK,
				Name:  "gather.v6.result",
				Msg:   "ipv6 candidates gathered",
				KVs: map[string]any{
					"count": len(v6Addrs),
				},
			})
		}
	}

	// 2) Port mapping helpers (best-effort; does not block exchange).
	type portmapState struct {
		mu        sync.Mutex
		frozen    bool
		included  []netip.AddrPort
		doneCount int
	}
	var (
		pmState        *portmapState
		pmSnapshotDone chan struct{}
		pmUpdateCh     chan struct{}
		pmStart        time.Time
		pmDeadline     time.Time
	)
	if !cfg.DisablePortMap {
		pmState = &portmapState{}
		pmSnapshotDone = make(chan struct{})
		pmUpdateCh = make(chan struct{}, 1)
		pmStart = time.Now()
		pmDeadline = gatherStart.Add(cfg.GatherTimeout)

		emit(obs.Event{Stage: obs.StageGather, Kind: obs.KindStart, Name: "gather.portmap.start", Msg: "portmap start"})

		emitOutcome := func(res PortMapAttemptResult, included bool) {
			evKind := obs.KindOK
			msg := "portmap method ok"
			errText := ""
			if res.Err != nil {
				evKind = obs.KindInfo
				msg = "portmap method finished with error"
				errText = res.Err.Error()
			}
			emit(obs.Event{
				Stage: obs.StageGather,
				Kind:  evKind,
				Name:  "gather.portmap.method.result",
				Msg:   msg,
				Err:   errText,
				KVs: map[string]any{
					"method":               res.Method,
					"included_in_snapshot": included,
					"count":                len(res.Candidates),
					"ms":                   res.Duration.Milliseconds(),
				},
			})
		}

		run := func(fn portMapperFunc) {
			res, cleanup := fn(ctx, udp4Port, cfg.SessionLease)

			included := time.Now().Before(pmDeadline)
			added := false
			pmState.mu.Lock()
			if pmState.frozen {
				included = false
			} else if included {
				if len(res.Candidates) > 0 {
					pmState.included = append(pmState.included, res.Candidates...)
					added = true
				}
			}
			pmState.doneCount++
			pmState.mu.Unlock()
			if added {
				select {
				case pmUpdateCh <- struct{}{}:
				default:
				}
			}

			emitOutcome(res, included)

			<-ctx.Done()
			if cleanup == nil {
				return
			}
			cctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
			defer cancel()
			if err := cleanup(cctx); err != nil {
				emit(obs.Event{Stage: obs.StageGather, Kind: obs.KindInfo, Name: "gather.portmap.unmap.error", Msg: "portmap unmap error", Err: err.Error(), KVs: map[string]any{"method": res.Method}})
				return
			}
			emit(obs.Event{Stage: obs.StageGather, Kind: obs.KindInfo, Name: "gather.portmap.unmap.ok", Msg: "portmap unmapped", KVs: map[string]any{"method": res.Method}})
		}

		go run(portmapUPnP)
		go run(portmapNATPMP)

		go func() {
			delay := time.Until(pmDeadline)
			if delay < 0 {
				delay = 0
			}
			timer := time.NewTimer(delay)
			defer timer.Stop()

			reason := "timeout"
			select {
			case <-timer.C:
			case <-pmSnapshotDone:
				reason = "snapshot_closed"
			case <-ctx.Done():
				reason = "session_done"
			}

			pmState.mu.Lock()
			done := pmState.doneCount
			included := append([]netip.AddrPort(nil), pmState.included...)
			pmState.mu.Unlock()

			included = TrimDirectAddrPorts(included)

			emit(obs.Event{
				Stage: obs.StageGather,
				Kind:  obs.KindInfo,
				Name:  "gather.portmap.cutoff",
				Msg:   "portmap snapshot cutoff",
				KVs: map[string]any{
					"reason":       reason,
					"ms":           time.Since(pmStart).Milliseconds(),
					"included":     len(included) > 0,
					"direct_v4":    len(included),
					"methods_done": done,
				},
			})
		}()
	}

	// 3) STUN gating rule A.
	var mappedAddrs []string
	if len(cfg.StunServers) == 0 {
		emit(obs.Event{
			Stage: obs.StageGather,
			Kind:  obs.KindInfo,
			Name:  "gather.stun.skip",
			Msg:   "stun not configured; punching disabled for this session",
		})
	} else {
		emit(obs.Event{Stage: obs.StageGather, Kind: obs.KindStart, Name: "gather.stun.start", Msg: "stun start"})

		stunCtx, cancel := context.WithTimeout(ctx, cfg.StunTimeout)
		defer cancel()
		start := time.Now()
		stunRes := DiscoverSTUN(stunCtx, udp4Conn, cfg.StunServers)
		mappedAddrs = stunRes.MappedAddrs

		kind := obs.KindOK
		msg := "stun ok"
		errText := ""
		if len(stunRes.Errors) > 0 {
			kind = obs.KindInfo
			msg = "stun finished with errors"
			errText = fmt.Sprintf("%v", stunRes.Errors)
		}

		emit(obs.Event{
			Stage: obs.StageGather,
			Kind:  kind,
			Name:  "gather.stun.result",
			Msg:   msg,
			Err:   errText,
			KVs: map[string]any{
				"count": len(mappedAddrs),
				"ms":    time.Since(start).Milliseconds(),
			},
		})
	}

	assistedAddrs := make([]string, 0)
	if !cfg.DisableAssistedAddrs {
		localIPs, _ := nathole.ListLocalIPsForNatHole(10)
		assistedAddrs = make([]string, 0, len(localIPs))
		for _, ip := range localIPs {
			assistedAddrs = append(assistedAddrs, net.JoinHostPort(ip, strconv.Itoa(udp4Port)))
		}
	}

	// In sessions without STUN and without IPv6 direct candidates, portmap is the
	// only remaining source of exchangeable candidates. Wait (bounded) for the
	// first portmap result to avoid a racy "no candidates" failure.
	if pmUpdateCh != nil && len(cfg.StunServers) == 0 && len(directCandidates) == 0 {
		pmState.mu.Lock()
		needWait := len(pmState.included) == 0 && !pmState.frozen
		pmState.mu.Unlock()

		if needWait {
			wait := time.Until(pmDeadline)
			if wait > 0 {
				emit(obs.Event{
					Stage: obs.StageGather,
					Kind:  obs.KindInfo,
					Name:  "gather.portmap.wait",
					Msg:   "waiting for portmap candidates",
					KVs: map[string]any{
						"ms": wait.Milliseconds(),
					},
				})
				timer := time.NewTimer(wait)
				select {
				case <-pmUpdateCh:
				case <-timer.C:
				case <-ctx.Done():
				}
				timer.Stop()
			}
		}
	}

	if pmSnapshotDone != nil {
		pmState.mu.Lock()
		pmState.frozen = true
		included := append([]netip.AddrPort(nil), pmState.included...)
		done := pmState.doneCount
		pmState.mu.Unlock()
		close(pmSnapshotDone)

		included = TrimDirectAddrPorts(included)
		directCandidates = append(directCandidates, included...)

		emit(obs.Event{
			Stage: obs.StageGather,
			Kind:  obs.KindInfo,
			Name:  "gather.portmap.snapshot",
			Msg:   "portmap snapshot finalized",
			KVs: map[string]any{
				"included":     len(included) > 0,
				"direct_v4":    len(included),
				"methods_done": done,
				"deadline_ms":  cfg.GatherTimeout.Milliseconds(),
			},
		})
	}
	directAddrs := TrimAndFormatDirectAddrs(directCandidates)
	if len(directAddrs) == 0 && len(mappedAddrs) == 0 {
		// Nothing to exchange; keep it explicit to avoid silent hangs.
		return nil, errors.New("no candidates gathered: direct_addrs empty and stun mapped_addrs empty")
	}

	emit(obs.Event{
		Stage: obs.StageGather,
		Kind:  obs.KindOK,
		Name:  "gather.done",
		Msg:   "gather done",
		KVs: map[string]any{
			"direct_addrs": len(directAddrs),
			"mapped_addrs": len(mappedAddrs),
		},
	})

	return &GatherResult{
		UDP4Conn:      udp4Conn,
		UDP6Conn:      udp6Conn,
		DirectAddrs:   directAddrs,
		MappedAddrs:   mappedAddrs,
		AssistedAddrs: assistedAddrs,
	}, nil
}

func bindUDP4(requestedPort int) (*net.UDPConn, int, error) {
	laddr := &net.UDPAddr{IP: net.IPv4zero, Port: requestedPort}
	conn, err := net.ListenUDP("udp4", laddr)
	if err != nil {
		return nil, 0, fmt.Errorf("bind udp4 port %d: %w", requestedPort, err)
	}
	ua, ok := conn.LocalAddr().(*net.UDPAddr)
	if !ok {
		_ = conn.Close()
		return nil, 0, errors.New("udp4 local addr is not UDPAddr")
	}
	return conn, ua.Port, nil
}

func bindUDP6(requestedPort int, udp4Port int) (*net.UDPConn, int, error) {
	// Best-effort same port only when user pinned a port.
	port := 0
	if requestedPort > 0 {
		port = requestedPort
	}

	try := func(p int) (*net.UDPConn, int, error) {
		laddr := &net.UDPAddr{IP: net.IPv6zero, Port: p}
		conn, err := net.ListenUDP("udp6", laddr)
		if err != nil {
			return nil, 0, err
		}
		ua, ok := conn.LocalAddr().(*net.UDPAddr)
		if !ok {
			_ = conn.Close()
			return nil, 0, errors.New("udp6 local addr is not UDPAddr")
		}
		return conn, ua.Port, nil
	}

	conn, port6, err := try(port)
	if err == nil {
		return conn, port6, nil
	}
	if requestedPort > 0 {
		// If binding the pinned port fails, allow fallback to a random port.
		return try(0)
	}
	// If port wasn't pinned, failure just disables udp6 direct for this session.
	return nil, 0, err
}
