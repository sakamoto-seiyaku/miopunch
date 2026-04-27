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
	"github.com/miopunch/miopunch/internal/stunclient"
	"github.com/miopunch/miopunch/internal/wire"
	"github.com/miopunch/miopunch/nat"
)

type GatherConfig struct {
	ListenPort int

	P2PNetwork P2PNetwork

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

	TCPBasePort   int
	TCPListenPort int
	TCP4Listener  *net.TCPListener
	TCP6Listener  *net.TCPListener

	DirectAddrs   []string
	MappedAddrs   []string
	AssistedAddrs []string

	TCPDirectAddrs   []string
	TCPAssistedAddrs []string
	TCPMappedAddrs   []string
	TCPSTUNCN        *wire.STUNViewObservation
	TCPSTUNGlobal    *wire.STUNViewObservation

	STUNCN     *wire.STUNViewObservation
	STUNGlobal *wire.STUNViewObservation
}

func Gather(ctx context.Context, sid string, cfg GatherConfig) (res *GatherResult, retErr error) {
	network, err := ParseP2PNetwork(string(cfg.P2PNetwork))
	if err != nil {
		return nil, err
	}
	cfg.P2PNetwork = network

	family, err := ParseP2PIPFamily(string(cfg.P2PIPFamily))
	if err != nil {
		return nil, err
	}
	cfg.P2PIPFamily = family

	allowV4 := cfg.P2PIPFamily != P2PIPFamilyV6
	allowV6 := cfg.P2PIPFamily != P2PIPFamilyV4
	allowTCP := cfg.P2PNetwork != P2PNetworkUDPOnly
	allowUDP := cfg.P2PNetwork != P2PNetworkTCPOnly

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
	if allowUDP && allowV4 {
		udp4Conn, udp4Port, err = bindUDP4(cfg.ListenPort)
		if err != nil {
			return nil, err
		}
	}

	var udp6Conn *net.UDPConn
	var udp6Port int
	if allowUDP && allowV6 {
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

	var tcpBasePort int
	var tcpListenPort int
	var tcp4Listener *net.TCPListener
	var tcp6Listener *net.TCPListener

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
		if tcp4Listener != nil {
			_ = tcp4Listener.Close()
		}
		if tcp6Listener != nil {
			_ = tcp6Listener.Close()
		}
	}()

	if allowTCP {
		pinned := cfg.ListenPort > 0

		preferredBase := cfg.ListenPort
		if !pinned {
			switch {
			case udp4Port > 0:
				preferredBase = udp4Port
			case udp6Port > 0:
				preferredBase = udp6Port
			default:
				preferredBase = 0
			}
		}

		listenAt := func(port int) (l4 *net.TCPListener, l6 *net.TCPListener, err error) {
			var err4, err6 error
			if allowV4 {
				l4, err4 = listenTCPWithReuseAddr("tcp4", net.JoinHostPort("0.0.0.0", strconv.Itoa(port)))
				if err4 != nil {
					emit(event.Event{
						Stage: event.StageGather,
						Kind:  event.KindInfo,
						Name:  "gather.tcp4.unavailable",
						Msg:   "tcp4 listen failed",
						Err:   err4.Error(),
						KVs: map[string]any{
							"listen_port": port,
						},
					})
				}
			}
			if allowV6 {
				l6, err6 = listenTCPWithReuseAddr("tcp6", net.JoinHostPort("::", strconv.Itoa(port)))
				if err6 != nil {
					emit(event.Event{
						Stage: event.StageGather,
						Kind:  event.KindInfo,
						Name:  "gather.tcp6.unavailable",
						Msg:   "tcp6 listen failed",
						Err:   err6.Error(),
						KVs: map[string]any{
							"listen_port": port,
						},
					})
				}
			}
			if l4 == nil && l6 == nil {
				if err4 != nil && err6 != nil {
					return nil, nil, fmt.Errorf("tcp listen failed: tcp4=%v tcp6=%v", err4, err6)
				}
				if err4 != nil {
					return nil, nil, err4
				}
				if err6 != nil {
					return nil, nil, err6
				}
				return nil, nil, errors.New("tcp listen failed")
			}
			return l4, l6, nil
		}

		tryPreferred := func() error {
			if preferredBase <= 0 || preferredBase > 65535 {
				return fmt.Errorf("invalid listen port: %d", preferredBase)
			}
			if preferredBase+100 > 65535 {
				return fmt.Errorf("invalid tcp listen port: %d (+100 exceeds 65535)", preferredBase)
			}

			p := preferredBase
			l := preferredBase + 100
			l4, l6, err := listenAt(l)
			if err != nil {
				return fmt.Errorf("tcp listen failed on %d: %w", l, err)
			}

			// Fail-fast for pinned ports: ensure the TCP base port is available for binding.
			if pinned && allowV4 {
				ln, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4zero, Port: p})
				if err != nil {
					if l4 != nil {
						_ = l4.Close()
					}
					if l6 != nil {
						_ = l6.Close()
					}
					return fmt.Errorf("tcp base port unavailable: %d: %w", p, err)
				}
				_ = ln.Close()
			}

			tcpBasePort = p
			tcpListenPort = l
			tcp4Listener = l4
			tcp6Listener = l6
			return nil
		}

		if err := tryPreferred(); err != nil {
			if pinned {
				return nil, err
			}

			// Probe a stable tcp listener port (L) and derive P=L-100.
			var primary *net.TCPListener
			var other *net.TCPListener
			if allowV6 {
				primary, err = listenTCPWithReuseAddr("tcp6", "[::]:0")
			} else if allowV4 {
				primary, err = listenTCPWithReuseAddr("tcp4", "0.0.0.0:0")
			} else {
				return nil, errors.New("tcp listen disabled by p2p ip family")
			}
			if err != nil {
				return nil, err
			}

			port := primary.Addr().(*net.TCPAddr).Port
			base := port - 100
			if base <= 0 {
				_ = primary.Close()
				return nil, fmt.Errorf("invalid tcp listen port: %d", port)
			}

			if allowV6 {
				tcp6Listener = primary
				if allowV4 {
					other, err = listenTCPWithReuseAddr("tcp4", net.JoinHostPort("0.0.0.0", strconv.Itoa(port)))
					if err != nil {
						emit(event.Event{
							Stage: event.StageGather,
							Kind:  event.KindInfo,
							Name:  "gather.tcp4.unavailable",
							Msg:   "tcp4 listen failed",
							Err:   err.Error(),
							KVs: map[string]any{
								"listen_port": port,
							},
						})
					}
					tcp4Listener = other
				}
			} else {
				tcp4Listener = primary
				if allowV6 {
					other, err = listenTCPWithReuseAddr("tcp6", net.JoinHostPort("::", strconv.Itoa(port)))
					if err != nil {
						emit(event.Event{
							Stage: event.StageGather,
							Kind:  event.KindInfo,
							Name:  "gather.tcp6.unavailable",
							Msg:   "tcp6 listen failed",
							Err:   err.Error(),
							KVs: map[string]any{
								"listen_port": port,
							},
						})
					}
					tcp6Listener = other
				}
			}

			tcpBasePort = base
			tcpListenPort = port
		}
	}

	emit(event.Event{
		Stage: event.StageGather,
		Kind:  event.KindStart,
		Name:  "gather.start",
		Msg:   "gather start",
		KVs: map[string]any{
			"p2p_ip_family": cfg.P2PIPFamily,
			"udp4_port":     udp4Port,
			"udp6_port":     udp6Port,
			"tcp_p":         tcpBasePort,
			"tcp_l":         tcpListenPort,
		},
	})

	gatherStart := time.Now()

	localIPs, _ := nat.ListLocalIPsForNatHole(10)

	// 1) Local candidates (non-blocking).
	var (
		v6Addrs             []netip.Addr
		directCandidates    []netip.AddrPort
		tcpDirectCandidates []netip.AddrPort
		tcpAssistedAddrs    []string
	)
	if allowV6 && (udp6Conn != nil || tcp6Listener != nil) {
		v6Addrs, err = GatherLocalIPv6Candidates()
		if err != nil {
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindFail,
				Name:  "gather.v6.error",
				Msg:   "ipv6 gather failed",
				Err:   err.Error(),
			})
		} else {
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
	if allowUDP && udp6Conn != nil && udp6Port > 0 {
		for _, addr := range v6Addrs {
			directCandidates = append(directCandidates, netip.AddrPortFrom(addr, uint16(udp6Port)))
		}
	}
	if tcp6Listener != nil && tcpListenPort > 0 {
		for _, addr := range v6Addrs {
			tcpDirectCandidates = append(tcpDirectCandidates, netip.AddrPortFrom(addr, uint16(tcpListenPort)))
		}
	}
	if tcp4Listener != nil && tcpListenPort > 0 {
		buckets := classifyTCP4ListenCandidates(localIPs, tcpListenPort)
		tcpDirectCandidates = append(tcpDirectCandidates, buckets.DirectAddrs...)
		if !cfg.DisableAssistedAddrs {
			tcpAssistedAddrs = append(tcpAssistedAddrs, buckets.AssistedAddrs...)
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
	if allowUDP && allowV4 && udp4Conn != nil && !cfg.DisablePortMap {
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

	var (
		tcpPMState        *portmapState
		tcpPMSnapshotDone chan struct{}
		tcpPMUpdateCh     chan struct{}
		tcpPMStart        time.Time
		tcpPMDeadline     time.Time
	)
	if allowTCP && allowV4 && tcp4Listener != nil && tcpListenPort > 0 && !cfg.DisablePortMap {
		tcpPMState = &portmapState{}
		tcpPMSnapshotDone = make(chan struct{})
		tcpPMUpdateCh = make(chan struct{}, 1)
		tcpPMStart = time.Now()
		tcpPMDeadline = gatherStart.Add(cfg.GatherTimeout)

		emit(event.Event{Stage: event.StageGather, Kind: event.KindStart, Name: "gather.tcp_portmap.start", Msg: "tcp portmap start"})

		emitOutcome := func(res PortMapAttemptResult, included bool) {
			evKind := event.KindOK
			msg := "tcp portmap method ok"
			errText := ""
			if res.Err != nil {
				evKind = event.KindInfo
				msg = "tcp portmap method finished with error"
				errText = res.Err.Error()
			}
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  evKind,
				Name:  "gather.tcp_portmap.method.result",
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
			res, cleanup := fn(ctx, tcpListenPort, cfg.SessionLease)

			included := time.Now().Before(tcpPMDeadline)
			added := false
			tcpPMState.mu.Lock()
			if tcpPMState.frozen {
				included = false
			} else if included {
				if len(res.Candidates) > 0 {
					tcpPMState.included = append(tcpPMState.included, res.Candidates...)
					added = true
				}
			}
			tcpPMState.doneCount++
			tcpPMState.mu.Unlock()
			if added {
				select {
				case tcpPMUpdateCh <- struct{}{}:
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
				emit(event.Event{Stage: event.StageGather, Kind: event.KindInfo, Name: "gather.tcp_portmap.unmap.error", Msg: "tcp portmap unmap error", Err: err.Error(), KVs: map[string]any{"method": res.Method}})
				return
			}
			emit(event.Event{Stage: event.StageGather, Kind: event.KindInfo, Name: "gather.tcp_portmap.unmap.ok", Msg: "tcp portmap unmapped", KVs: map[string]any{"method": res.Method}})
		}

		go run(portmapUPnPTCP)
		go run(portmapNATPMPTCP)

		go func() {
			delay := time.Until(tcpPMDeadline)
			if delay < 0 {
				delay = 0
			}
			timer := time.NewTimer(delay)
			defer timer.Stop()

			reason := "timeout"
			select {
			case <-timer.C:
			case <-tcpPMSnapshotDone:
				reason = "snapshot_closed"
			case <-ctx.Done():
				reason = "session_done"
			}

			tcpPMState.mu.Lock()
			done := tcpPMState.doneCount
			included := append([]netip.AddrPort(nil), tcpPMState.included...)
			tcpPMState.mu.Unlock()

			included = TrimDirectAddrPorts(included)

			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindInfo,
				Name:  "gather.tcp_portmap.cutoff",
				Msg:   "tcp portmap snapshot cutoff",
				KVs: map[string]any{
					"reason":       reason,
					"ms":           time.Since(tcpPMStart).Milliseconds(),
					"included":     len(included) > 0,
					"direct_v4":    len(included),
					"methods_done": done,
				},
			})
		}()
	}

	// 3) STUN gating rule A.
	var (
		mappedAddrs        []string
		stunCN, stunGlobal *wire.STUNViewObservation

		tcpMappedAddrs           []string
		tcpSTUNCN, tcpSTUNGlobal *wire.STUNViewObservation
	)
	if !allowV4 {
		emit(event.Event{
			Stage: event.StageGather,
			Kind:  event.KindInfo,
			Name:  "gather.stun.skip",
			Msg:   "stun disabled by p2p ip family",
		})
		emit(event.Event{
			Stage: event.StageGather,
			Kind:  event.KindInfo,
			Name:  "gather.tcp_stun.skip",
			Msg:   "tcp stun disabled by p2p ip family",
		})
	} else {
		resolver, err := netutil.NewDNSResolver(cfg.BuiltinDNSMode, cfg.BuiltinDNSServers)
		if err != nil {
			return nil, err
		}

		stunCtx, cancel := context.WithTimeout(ctx, cfg.StunTimeout)
		defer cancel()

		var wg sync.WaitGroup

		if allowUDP && udp4Conn != nil {
			wg.Add(1)
			go func() {
				defer wg.Done()

				emit(event.Event{Stage: event.StageGather, Kind: event.KindStart, Name: "gather.stun.start", Msg: "stun start"})

				discoverExplicit := func(servers []string) (*STUNDiscoveryResult, error) {
					usable, ignored, filterErrors := stunclient.FilterHostPorts(servers, stunclient.EndpointSchemeUDP)
					if len(usable) == 0 {
						return nil, fmt.Errorf("stun configured but no usable udp endpoints (ignored=%v errors=%v)", ignored, filterErrors)
					}

					resolved, resolveErrors := stunclient.ResolveHostPortsIP4(stunCtx, resolver, usable, 0)
					stunRes := DiscoverSTUN(stunCtx, udp4Conn, resolved)
					stunRes.Errors = append(stunRes.Errors, filterErrors...)
					if len(ignored) > 0 {
						stunRes.Errors = append(stunRes.Errors, fmt.Sprintf("ignored non-udp stun endpoints: %v", ignored))
					}
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
						return
					}
					start := time.Now()
					stunRes, err := discoverExplicit(cfg.StunServers)
					if err != nil {
						emit(event.Event{
							Stage: event.StageGather,
							Kind:  event.KindInfo,
							Name:  "gather.stun.error",
							Msg:   "stun failed",
							Err:   err.Error(),
						})
						return
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
						emit(event.Event{
							Stage: event.StageGather,
							Kind:  event.KindInfo,
							Name:  "gather.stun.error",
							Msg:   "stun failed",
							Err:   err.Error(),
						})
						return
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
					client := stunclient.NewUDPClient(udp4Conn)
					defer client.Close()

					var (
						cnObs     *wire.STUNViewObservation
						globalObs *wire.STUNViewObservation
						wg2       sync.WaitGroup
					)
					wg2.Add(2)
					go func() {
						defer wg2.Done()
						globalObs = observeInternalSTUNView(stunCtx, client, resolver, globalServers, localIPs)
					}()
					go func() {
						defer wg2.Done()
						cnObs = observeInternalSTUNView(stunCtx, client, resolver, cnServers, localIPs)
					}()
					wg2.Wait()
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
			}()
		} else if !allowUDP {
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindInfo,
				Name:  "gather.stun.skip",
				Msg:   "stun disabled by p2p_network",
			})
		} else {
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindInfo,
				Name:  "gather.stun.skip",
				Msg:   "stun disabled by udp4 availability",
			})
		}

		if allowTCP && tcpBasePort > 0 {
			wg.Add(1)
			go func() {
				defer wg.Done()

				emit(event.Event{Stage: event.StageGather, Kind: event.KindStart, Name: "gather.tcp_stun.start", Msg: "tcp stun start"})

				dialer := &net.Dialer{LocalAddr: &net.TCPAddr{Port: tcpBasePort}}

				discoverExplicit := func(servers []string) (*STUNDiscoveryResult, error) {
					resolved, ignored, errs := resolveSTUNServersTCP(stunCtx, resolver, servers)
					if len(resolved) == 0 {
						return nil, fmt.Errorf("stun configured but no usable tcp endpoints (ignored=%v errors=%v)", ignored, errs)
					}

					stunRes := DiscoverSTUNTCP(stunCtx, dialer, resolved)
					stunRes.Errors = append(stunRes.Errors, errs...)
					if len(ignored) > 0 {
						stunRes.Errors = append(stunRes.Errors, fmt.Sprintf("ignored non-tcp stun endpoints: %v", ignored))
					}
					return &stunRes, nil
				}

				switch {
				case cfg.StunExplicit:
					if len(cfg.StunServers) == 0 {
						emit(event.Event{
							Stage: event.StageGather,
							Kind:  event.KindInfo,
							Name:  "gather.tcp_stun.skip",
							Msg:   "stun explicitly disabled; tcp punching disabled for this session",
						})
						return
					}
					start := time.Now()
					stunRes, err := discoverExplicit(cfg.StunServers)
					if err != nil {
						emit(event.Event{
							Stage: event.StageGather,
							Kind:  event.KindInfo,
							Name:  "gather.tcp_stun.error",
							Msg:   "tcp stun failed",
							Err:   err.Error(),
						})
						return
					}
					tcpMappedAddrs = stunRes.MappedAddrs

					kind := event.KindOK
					msg := "tcp stun ok"
					errText := ""
					if len(stunRes.Errors) > 0 {
						kind = event.KindInfo
						msg = "tcp stun finished with errors"
						errText = fmt.Sprintf("%v", stunRes.Errors)
					}
					emit(event.Event{
						Stage: event.StageGather,
						Kind:  kind,
						Name:  "gather.tcp_stun.result",
						Msg:   msg,
						Err:   errText,
						KVs: map[string]any{
							"configured": true,
							"count":      len(tcpMappedAddrs),
							"ok_count":   stunRes.OkCount,
							"rtt_ms":     stunRes.RTTMs,
							"ms":         time.Since(start).Milliseconds(),
						},
					})
				case len(cfg.StunServers) > 0:
					start := time.Now()
					stunRes, err := discoverExplicit(cfg.StunServers)
					if err != nil {
						emit(event.Event{
							Stage: event.StageGather,
							Kind:  event.KindInfo,
							Name:  "gather.tcp_stun.error",
							Msg:   "tcp stun failed",
							Err:   err.Error(),
						})
						return
					}
					tcpMappedAddrs = stunRes.MappedAddrs
					emit(event.Event{
						Stage: event.StageGather,
						Kind:  event.KindOK,
						Name:  "gather.tcp_stun.result",
						Msg:   "tcp stun ok",
						KVs: map[string]any{
							"configured": true,
							"count":      len(tcpMappedAddrs),
							"ok_count":   stunRes.OkCount,
							"rtt_ms":     stunRes.RTTMs,
							"ms":         time.Since(start).Milliseconds(),
						},
					})
				default:
					start := time.Now()
					cnServers, globalServers := internalSTUNBuckets()

					cnObs := observeInternalSTUNViewTCP(stunCtx, dialer, resolver, cnServers, localIPs)
					globalObs := observeInternalSTUNViewTCP(stunCtx, dialer, resolver, globalServers, localIPs)
					tcpSTUNCN = cnObs
					tcpSTUNGlobal = globalObs
					tcpMappedAddrs = append(tcpMappedAddrs, cnObs.MappedAddrs...)
					tcpMappedAddrs = append(tcpMappedAddrs, globalObs.MappedAddrs...)
					tcpMappedAddrs = slices.Compact(tcpMappedAddrs)

					emit(event.Event{
						Stage: event.StageGather,
						Kind:  event.KindInfo,
						Name:  "gather.tcp_stun.view.result",
						Msg:   "tcp stun cn observation",
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
						Name:  "gather.tcp_stun.view.result",
						Msg:   "tcp stun global observation",
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
						Name:  "gather.tcp_stun.result",
						Msg:   "tcp stun sampled",
						KVs: map[string]any{
							"configured": false,
							"ms":         time.Since(start).Milliseconds(),
						},
					})
				}
			}()
		} else if allowTCP {
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindInfo,
				Name:  "gather.tcp_stun.skip",
				Msg:   "tcp stun disabled by tcp base port",
			})
		} else {
			emit(event.Event{
				Stage: event.StageGather,
				Kind:  event.KindInfo,
				Name:  "gather.tcp_stun.skip",
				Msg:   "tcp stun disabled by p2p_network",
			})
		}

		wg.Wait()
	}

	assistedAddrs := make([]string, 0)
	if allowV4 && !cfg.DisableAssistedAddrs {
		assistedPort := 0
		if udp4Conn != nil && udp4Port > 0 {
			assistedPort = udp4Port
		} else if tcpListenPort > 0 {
			assistedPort = tcpListenPort
		}
		if assistedPort > 0 {
			assistedAddrs = make([]string, 0, len(localIPs))
			for _, ip := range localIPs {
				assistedAddrs = append(assistedAddrs, net.JoinHostPort(ip, strconv.Itoa(assistedPort)))
			}
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

	if tcpPMUpdateCh != nil && len(cfg.StunServers) == 0 {
		tcpPMState.mu.Lock()
		needWait := len(tcpPMState.included) == 0 && !tcpPMState.frozen
		tcpPMState.mu.Unlock()

		if needWait {
			wait := time.Until(tcpPMDeadline)
			if wait > 0 {
				emit(event.Event{
					Stage: event.StageGather,
					Kind:  event.KindInfo,
					Name:  "gather.tcp_portmap.wait",
					Msg:   "waiting for tcp portmap candidates",
					KVs: map[string]any{
						"ms": wait.Milliseconds(),
					},
				})
				timer := time.NewTimer(wait)
				select {
				case <-tcpPMUpdateCh:
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

	if tcpPMSnapshotDone != nil {
		tcpPMState.mu.Lock()
		tcpPMState.frozen = true
		included := append([]netip.AddrPort(nil), tcpPMState.included...)
		done := tcpPMState.doneCount
		tcpPMState.mu.Unlock()
		close(tcpPMSnapshotDone)

		included, dropped := filterTCPPortmapDirectAddrs(included)
		included = TrimDirectAddrPorts(included)
		tcpDirectCandidates = append(tcpDirectCandidates, included...)

		emit(event.Event{
			Stage: event.StageGather,
			Kind:  event.KindInfo,
			Name:  "gather.tcp_portmap.snapshot",
			Msg:   "tcp portmap snapshot finalized",
			KVs: map[string]any{
				"included":     len(included) > 0,
				"direct_v4":    len(included),
				"dropped":      len(dropped),
				"methods_done": done,
				"deadline_ms":  cfg.GatherTimeout.Milliseconds(),
			},
		})
	}
	directAddrs := TrimAndFormatDirectAddrs(directCandidates)
	tcpDirectAddrs := TrimAndFormatDirectAddrs(tcpDirectCandidates)

	udpExchangePossible := len(directAddrs) > 0 || len(mappedAddrs) > 0 || len(assistedAddrs) > 0
	tcpExchangePossible := len(tcpDirectAddrs) > 0 || len(tcpMappedAddrs) > 0 || len(tcpAssistedAddrs) > 0

	switch {
	case allowUDP && allowTCP:
		if !udpExchangePossible && !tcpExchangePossible {
			// Nothing to exchange; keep it explicit to avoid silent hangs.
			return nil, errors.New("no candidates gathered: udp and tcp snapshots both empty")
		}
	case allowUDP:
		if !udpExchangePossible {
			return nil, errors.New("no candidates gathered: udp snapshot empty")
		}
	case allowTCP:
		if !tcpExchangePossible {
			return nil, errors.New("no candidates gathered: tcp snapshot empty")
		}
	default:
		return nil, errors.New("no candidates gathered: both udp and tcp disabled by p2p_network")
	}

	emit(event.Event{
		Stage: event.StageGather,
		Kind:  event.KindOK,
		Name:  "gather.done",
		Msg:   "gather done",
		KVs: map[string]any{
			"direct_addrs":       len(directAddrs),
			"mapped_addrs":       len(mappedAddrs),
			"tcp_direct_addrs":   len(tcpDirectAddrs),
			"tcp_mapped_addrs":   len(tcpMappedAddrs),
			"tcp_assisted_addrs": len(tcpAssistedAddrs),
		},
	})

	return &GatherResult{
		UDP4Conn:         udp4Conn,
		UDP6Conn:         udp6Conn,
		TCPBasePort:      tcpBasePort,
		TCPListenPort:    tcpListenPort,
		TCP4Listener:     tcp4Listener,
		TCP6Listener:     tcp6Listener,
		DirectAddrs:      directAddrs,
		MappedAddrs:      mappedAddrs,
		AssistedAddrs:    assistedAddrs,
		TCPDirectAddrs:   tcpDirectAddrs,
		TCPAssistedAddrs: tcpAssistedAddrs,
		TCPMappedAddrs:   tcpMappedAddrs,
		TCPSTUNCN:        tcpSTUNCN,
		TCPSTUNGlobal:    tcpSTUNGlobal,
		STUNCN:           stunCN,
		STUNGlobal:       stunGlobal,
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
