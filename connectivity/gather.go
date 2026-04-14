package connectivity

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"slices"
	"strconv"
	"sync"
	"time"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/netutil"
	"github.com/miopunch/miopunch/internal/wire"
	"github.com/miopunch/miopunch/nat"
)

type GatherConfig struct {
	ListenPort int

	P2PIPFamily P2PIPFamily

	DisableAssistedAddrs bool
	DisablePortMap       bool

	// StunServers is the user-provided STUN server list (host:port).
	StunServers []string
	// StunExplicit indicates the user explicitly configured STUN (including empty).
	// When true, internal STUN defaults and cn/global arbitration are disabled.
	StunExplicit bool

	BuiltinDNSMode    string
	BuiltinDNSServers []string

	StunTimeout time.Duration

	GatherTimeout time.Duration
	SessionLease  time.Duration

	Emitter *event.Emitter
}

type GatherResult struct {
	UDP4Conn *net.UDPConn
	UDP6Conn *net.UDPConn

	DirectAddrs   []string
	MappedAddrs   []string
	AssistedAddrs []string

	STUNCN     *wire.STUNViewObservation
	STUNGlobal *wire.STUNViewObservation
}

func Gather(ctx context.Context, sid string, cfg GatherConfig) (res *GatherResult, retErr error) {
	family, err := ParseP2PIPFamily(string(cfg.P2PIPFamily))
	if err != nil {
		return nil, err
	}
	cfg.P2PIPFamily = family

	allowV4 := cfg.P2PIPFamily != P2PIPFamilyV6
	allowV6 := cfg.P2PIPFamily != P2PIPFamilyV4

	if cfg.StunTimeout == 0 {
		cfg.StunTimeout = 3 * time.Second
	}
	if cfg.GatherTimeout == 0 {
		cfg.GatherTimeout = 1500 * time.Millisecond
	}
	if cfg.SessionLease == 0 {
		cfg.SessionLease = 5 * time.Minute
	}

	emit := func(ev event.Event) {
		if cfg.Emitter != nil {
			ev.SID = sid
			cfg.Emitter.Emit(ev)
		}
	}

	var udp4Conn *net.UDPConn
	var udp4Port int
	if allowV4 {
		udp4Conn, udp4Port, err = bindUDP4(cfg.ListenPort)
		if err != nil {
			return nil, err
		}
	}

	var udp6Conn *net.UDPConn
	var udp6Port int
	if allowV6 {
		udp6Conn, udp6Port, err = bindUDP6(cfg.ListenPort, udp4Port)
		if err != nil {
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindInfo,
				Name:  "gather.udp6.unavailable",
				Msg:   "udp6 bind failed",
				Err:   err.Error(),
			})
		}
	}

	defer func() {
		if retErr == nil {
			return
		}
		if udp4Conn != nil {
			_ = udp4Conn.Close()
		}
		if udp6Conn != nil {
			_ = udp6Conn.Close()
		}
	}()

	emit(event.Event{
		Stage: event.StageGather,
		Kind:  event.KindStart,
		Name:  "gather.start",
		Msg:   "gather start",
		KVs: map[string]any{
			"p2p_ip_family": cfg.P2PIPFamily,
			"udp4_port":     udp4Port,
			"udp6_port":     udp6Port,
		},
	})

	gatherStart := time.Now()

	// 1) IPv6 local candidates (non-blocking).
	var directCandidates []netip.AddrPort
	if udp6Conn != nil {
		v6Addrs, err := GatherLocalIPv6Candidates()
		if err != nil {
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindFail,
				Name:  "gather.v6.error",
				Msg:   "ipv6 gather failed",
				Err:   err.Error(),
			})
		} else {
			for _, addr := range v6Addrs {
				directCandidates = append(directCandidates, netip.AddrPortFrom(addr, uint16(udp6Port)))
			}
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindOK,
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
	if allowV4 && udp4Conn != nil && !cfg.DisablePortMap {
		pmState = &portmapState{}
		pmSnapshotDone = make(chan struct{})
		pmUpdateCh = make(chan struct{}, 1)
		pmStart = time.Now()
		pmDeadline = gatherStart.Add(cfg.GatherTimeout)

		emit(event.Event{Stage: event.StageGather, Kind: event.KindStart, Name: "gather.portmap.start", Msg: "portmap start"})

		emitOutcome := func(res PortMapAttemptResult, included bool) {
			evKind := event.KindOK
			msg := "portmap method ok"
			errText := ""
			if res.Err != nil {
				evKind = event.KindInfo
				msg = "portmap method finished with error"
				errText = res.Err.Error()
			}
			emit(event.Event{
				Stage: event.StageGather,
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
				emit(event.Event{Stage: event.StageGather, Kind: event.KindInfo, Name: "gather.portmap.unmap.error", Msg: "portmap unmap error", Err: err.Error(), KVs: map[string]any{"method": res.Method}})
				return
			}
			emit(event.Event{Stage: event.StageGather, Kind: event.KindInfo, Name: "gather.portmap.unmap.ok", Msg: "portmap unmapped", KVs: map[string]any{"method": res.Method}})
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

			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindInfo,
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

	localIPs, _ := nat.ListLocalIPsForNatHole(10)

	// 3) STUN gating rule A.
	var mappedAddrs []string
	var stunCN, stunGlobal *wire.STUNViewObservation
	if !allowV4 || udp4Conn == nil {
		emit(event.Event{
			Stage: event.StageGather,
			Kind:  event.KindInfo,
			Name:  "gather.stun.skip",
			Msg:   "stun disabled by p2p ip family",
		})
	} else {
		resolver, err := netutil.NewDNSResolver(cfg.BuiltinDNSMode, cfg.BuiltinDNSServers)
		if err != nil {
			return nil, err
		}

		emit(event.Event{Stage: event.StageGather, Kind: event.KindStart, Name: "gather.stun.start", Msg: "stun start"})

		stunCtx, cancel := context.WithTimeout(ctx, cfg.StunTimeout)
		defer cancel()

		discoverExplicit := func(servers []string) (*STUNDiscoveryResult, error) {
			resolved, resolveErrors := resolveSTUNServers(stunCtx, resolver, servers)
			stunRes := DiscoverSTUN(stunCtx, udp4Conn, resolved)
			stunRes.Errors = append(stunRes.Errors, resolveErrors...)
			return &stunRes, nil
		}

		switch {
		case cfg.StunExplicit:
			if len(cfg.StunServers) == 0 {
				emit(event.Event{
					Stage: event.StageGather,
					Kind:  event.KindInfo,
					Name:  "gather.stun.skip",
					Msg:   "stun explicitly disabled; punching disabled for this session",
				})
				break
			}
			start := time.Now()
			stunRes, err := discoverExplicit(cfg.StunServers)
			if err != nil {
				return nil, err
			}
			mappedAddrs = stunRes.MappedAddrs

			kind := event.KindOK
			msg := "stun ok"
			errText := ""
			if len(stunRes.Errors) > 0 {
				kind = event.KindInfo
				msg = "stun finished with errors"
				errText = fmt.Sprintf("%v", stunRes.Errors)
			}
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  kind,
				Name:  "gather.stun.result",
				Msg:   msg,
				Err:   errText,
				KVs: map[string]any{
					"configured": true,
					"count":      len(mappedAddrs),
					"ok_count":   stunRes.OkCount,
					"rtt_ms":     stunRes.RTTMs,
					"ms":         time.Since(start).Milliseconds(),
				},
			})
		case len(cfg.StunServers) > 0:
			start := time.Now()
			stunRes, err := discoverExplicit(cfg.StunServers)
			if err != nil {
				return nil, err
			}
			mappedAddrs = stunRes.MappedAddrs
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindOK,
				Name:  "gather.stun.result",
				Msg:   "stun ok",
				KVs: map[string]any{
					"configured": true,
					"count":      len(mappedAddrs),
					"ok_count":   stunRes.OkCount,
					"rtt_ms":     stunRes.RTTMs,
					"ms":         time.Since(start).Milliseconds(),
				},
			})
		default:
			// P3.5: internal STUN cn/global sampling (best-effort). Selection happens in exchange.
			start := time.Now()
			cnServers, globalServers := internalSTUNBuckets()

			// Prefer sampling global first so the default/fallback view is always measured.
			globalObs := observeSTUNView(stunCtx, udp4Conn, resolver, globalServers, localIPs)
			cnObs := observeSTUNView(stunCtx, udp4Conn, resolver, cnServers, localIPs)
			stunCN = cnObs
			stunGlobal = globalObs
			mappedAddrs = append(mappedAddrs, cnObs.MappedAddrs...)
			mappedAddrs = append(mappedAddrs, globalObs.MappedAddrs...)
			mappedAddrs = slices.Compact(mappedAddrs)

			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindInfo,
				Name:  "gather.stun.view.result",
				Msg:   "stun cn observation",
				KVs: map[string]any{
					"view":           "cn",
					"available":      cnObs.Available,
					"count":          len(cnObs.MappedAddrs),
					"ok_count":       cnObs.OkCount,
					"rtt_ms":         cnObs.RTTMs,
					"nat_difficulty": cnObs.NATDifficulty,
				},
			})
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindInfo,
				Name:  "gather.stun.view.result",
				Msg:   "stun global observation",
				KVs: map[string]any{
					"view":           "global",
					"available":      globalObs.Available,
					"count":          len(globalObs.MappedAddrs),
					"ok_count":       globalObs.OkCount,
					"rtt_ms":         globalObs.RTTMs,
					"nat_difficulty": globalObs.NATDifficulty,
				},
			})
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindOK,
				Name:  "gather.stun.result",
				Msg:   "stun sampled",
				KVs: map[string]any{
					"configured": false,
					"ms":         time.Since(start).Milliseconds(),
				},
			})
		}
	}

	assistedAddrs := make([]string, 0)
	if allowV4 && udp4Conn != nil && !cfg.DisableAssistedAddrs {
		assistedAddrs = make([]string, 0, len(localIPs))
		for _, ip := range localIPs {
			assistedAddrs = append(assistedAddrs, net.JoinHostPort(ip, strconv.Itoa(udp4Port)))
		}
	}

	// In sessions without STUN, portmap may still be the only viable IPv4
	// fallback after IPv6 direct fails. Wait (bounded) for the first portmap
	// result before freezing the snapshot so dual-stack no-STUN sessions can
	// include IPv4 direct candidates when they arrive in time.
	if pmUpdateCh != nil && len(cfg.StunServers) == 0 {
		pmState.mu.Lock()
		needWait := len(pmState.included) == 0 && !pmState.frozen
		pmState.mu.Unlock()

		if needWait {
			wait := time.Until(pmDeadline)
			if wait > 0 {
				emit(event.Event{
					Stage: event.StageGather,
					Kind:  event.KindInfo,
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

		emit(event.Event{
			Stage: event.StageGather,
			Kind:  event.KindInfo,
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

	emit(event.Event{
		Stage: event.StageGather,
		Kind:  event.KindOK,
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
		STUNCN:        stunCN,
		STUNGlobal:    stunGlobal,
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
