package connectivity

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"strings"
	"time"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/punching"
	"github.com/miopunch/miopunch/internal/wire"
)

type AttemptConfig struct {
	AttemptV6Timeout      time.Duration
	AttemptPortmapTimeout time.Duration

	P2PNetwork P2PNetwork

	P2PIPFamily P2PIPFamily

	DirectSendCount    int
	DirectSendInterval time.Duration

	Emitter *event.Emitter
}

type TCPConnOrigin string

const (
	TCPConnOriginDial   TCPConnOrigin = "dial"
	TCPConnOriginAccept TCPConnOrigin = "accept"
)

type TCPConn struct {
	Conn   net.Conn
	Origin TCPConnOrigin
}

type AttemptResult struct {
	Path   string
	Conn   *net.UDPConn
	Remote *net.UDPAddr

	// TCPConns is populated when the selected path is TCP. It may include multiple
	// simultaneously successful connections; the data plane is responsible for
	// selecting a single winner and closing non-winners.
	TCPConns []TCPConn
}

type PunchFunc func(ctx context.Context, listenConn *net.UDPConn, resp *wire.NatHoleResp, key []byte) (*net.UDPConn, *net.UDPAddr, error)

func Attempt(ctx context.Context, sid string, key []byte, udp4Conn *net.UDPConn, udp6Conn *net.UDPConn, tcp4Listener *net.TCPListener, tcp6Listener *net.TCPListener, resp *wire.NatHoleResp, cfg AttemptConfig) (*AttemptResult, error) {
	return attemptWithPunch(ctx, sid, key, udp4Conn, udp6Conn, tcp4Listener, tcp6Listener, resp, cfg, punching.MakeHole)
}

func attemptWithPunch(ctx context.Context, sid string, key []byte, udp4Conn *net.UDPConn, udp6Conn *net.UDPConn, tcp4Listener *net.TCPListener, tcp6Listener *net.TCPListener, resp *wire.NatHoleResp, cfg AttemptConfig, punch PunchFunc) (*AttemptResult, error) {
	if cfg.AttemptV6Timeout == 0 {
		cfg.AttemptV6Timeout = 800 * time.Millisecond
	}
	if cfg.AttemptPortmapTimeout == 0 {
		cfg.AttemptPortmapTimeout = 800 * time.Millisecond
	}
	if cfg.DirectSendCount <= 0 {
		cfg.DirectSendCount = 3
	}
	if cfg.DirectSendInterval == 0 {
		cfg.DirectSendInterval = 100 * time.Millisecond
	}

	emit := func(ev event.Event) {
		if cfg.Emitter != nil {
			ev.SID = sid
			cfg.Emitter.Emit(ev)
		}
	}

	network, err := ParseP2PNetwork(string(cfg.P2PNetwork))
	if err != nil {
		return nil, err
	}
	effectiveNetwork := network
	if strings.TrimSpace(resp.P2PNetwork) != "" {
		effectiveNetwork, err = ParseP2PNetwork(resp.P2PNetwork)
		if err != nil {
			return nil, fmt.Errorf("invalid resp p2p_network: %w", err)
		}
	}
	cfg.P2PNetwork = effectiveNetwork

	family, err := ParseP2PIPFamily(string(cfg.P2PIPFamily))
	if err != nil {
		return nil, err
	}
	cfg.P2PIPFamily = family

	allowV4 := cfg.P2PIPFamily != P2PIPFamilyV6
	allowV6 := cfg.P2PIPFamily != P2PIPFamilyV4
	allowTCP := cfg.P2PNetwork != P2PNetworkUDPOnly
	allowUDP := cfg.P2PNetwork != P2PNetworkTCPOnly

	if allowUDP && cfg.P2PIPFamily == P2PIPFamilyV6 && udp6Conn == nil {
		return nil, errors.New("udp6 conn is required for p2p ip family v6")
	}
	if allowUDP && allowV4 && udp4Conn == nil {
		return nil, errors.New("udp4 conn is required")
	}
	if resp == nil {
		return nil, errors.New("nil NatHoleResp")
	}
	if strings.TrimSpace(resp.Error) != "" {
		err := fmt.Errorf("exchange failed: %s", strings.TrimSpace(resp.Error))
		emit(event.Event{
			Stage: event.StageAttempt,
			Kind:  event.KindFail,
			Name:  "attempt.exchange.failed",
			Msg:   "exchange failed",
			Err:   err.Error(),
		})
		return nil, err
	}

	parsedUDP := ParseDirectAddrPorts(resp.PeerDirectAddrs)
	if len(parsedUDP.Invalid) > 0 {
		emit(event.Event{
			Stage: event.StageAttempt,
			Kind:  event.KindInfo,
			Name:  "attempt.peer_direct_addrs.invalid",
			Msg:   "invalid peer direct_addrs dropped",
			KVs: map[string]any{
				"count": len(parsedUDP.Invalid),
			},
		})
	}

	parsedTCP := ParseDirectAddrPorts(resp.PeerTCPDirectAddrs)
	if len(parsedTCP.Invalid) > 0 {
		emit(event.Event{
			Stage: event.StageAttempt,
			Kind:  event.KindInfo,
			Name:  "attempt.peer_tcp_direct_addrs.invalid",
			Msg:   "invalid peer tcp_direct_addrs dropped",
			KVs: map[string]any{
				"count": len(parsedTCP.Invalid),
			},
		})
	}

	peerUDPV6, peerUDPV4 := SplitAddrPortsByFamily(parsedUDP.Addrs)
	peerTCPV6, peerTCPV4 := SplitAddrPortsByFamily(parsedTCP.Addrs)
	emit(event.Event{
		Stage: event.StageAttempt,
		Kind:  event.KindStart,
		Name:  "attempt.start",
		Msg:   "attempt start",
		KVs: map[string]any{
			"p2p_network":   cfg.P2PNetwork,
			"p2p_ip_family": cfg.P2PIPFamily,
			"peer_v6":       len(peerUDPV6),
			"peer_v4":       len(peerUDPV4),
			"peer_tcp_v6":   len(peerTCPV6),
			"peer_tcp_v4":   len(peerTCPV4),
		},
	})

	if cfg.P2PIPFamily == P2PIPFamilyV6 && len(peerUDPV6) == 0 && len(peerTCPV6) == 0 {
		err := errors.New("p2p ip family v6 requires peer ipv6 candidates (tcp or udp)")
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindFail, Name: "attempt.v6.required", Msg: "ipv6-only requires peer ipv6 candidates", Err: err.Error()})
		return nil, err
	}

	var attemptErr error

	// Order: tcp6 -> tcp4 -> udp6 -> udp4.
	if allowTCP && allowV6 && tcp6Listener != nil && len(peerTCPV6) > 0 {
		res, err := attemptTCPDirect(ctx, sid, key, tcp6Listener, peerTCPV6, cfg, emit, "direct_tcp6")
		if err == nil && res != nil {
			return res, nil
		}
		attemptErr = err
	}

	if allowTCP && allowV4 && tcp4Listener != nil {
		if len(peerTCPV4) > 0 {
			res, err := attemptTCPDirect(ctx, sid, key, tcp4Listener, peerTCPV4, cfg, emit, "direct_tcp4")
			if err == nil && res != nil {
				return res, nil
			}
			attemptErr = err
		}

		res, err := attemptTCPPunching(ctx, sid, key, tcp4Listener, resp, cfg, emit)
		if err == nil && res != nil {
			return res, nil
		}
		if err != nil {
			attemptErr = err
		}
	}

	if allowUDP && allowV6 && udp6Conn != nil && len(peerUDPV6) > 0 {
		res, err := attemptUDPDirect(ctx, sid, key, udp6Conn, peerUDPV6, cfg, emit, "direct_ipv6")
		if err == nil && res != nil {
			return res, nil
		}
		attemptErr = err
	}

	if allowUDP && allowV4 && udp4Conn != nil && len(peerUDPV4) > 0 {
		res, err := attemptUDPDirect(ctx, sid, key, udp4Conn, peerUDPV4, cfg, emit, "direct_ipv4")
		if err == nil && res != nil {
			return res, nil
		}
		attemptErr = err
	}

	if cfg.P2PIPFamily == P2PIPFamilyV6 {
		if attemptErr == nil {
			attemptErr = errors.New("ipv6-only attempt failed")
		}
		return nil, attemptErr
	}

	if !allowUDP {
		if attemptErr == nil {
			attemptErr = errors.New("tcp-only attempt failed")
		}
		return nil, attemptErr
	}

	if !allowV4 || udp4Conn == nil {
		if attemptErr == nil {
			attemptErr = errors.New("udp4 conn is required for udp punching")
		}
		return nil, attemptErr
	}

	// UDP4 punching fallback (P1 kernel).
	return attemptUDPPunching(ctx, sid, key, udp4Conn, resp, cfg, emit, punch, attemptErr)
}

func attemptUDPDirect(ctx context.Context, sid string, key []byte, conn *net.UDPConn, candidates []netip.AddrPort, cfg AttemptConfig, emit func(event.Event), path string) (*AttemptResult, error) {
	if path == "direct_ipv6" {
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindStart, Name: "attempt.v6.start", Msg: "attempt ipv6 direct"})
	} else {
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindStart, Name: "attempt.v4.start", Msg: "attempt ipv4 direct"})
	}

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
	timeout := cfg.AttemptPortmapTimeout
	if path == "direct_ipv6" {
		timeout = cfg.AttemptV6Timeout
	}

	subCtx, cancel := context.WithTimeout(ctx, timeout)
	raddr, winner, err := directHandshakeFanout(subCtx, conn, sid, key, candidates, cfg.DirectSendCount, cfg.DirectSendInterval)
	cancel()
	if err == nil {
		for _, cand := range candidates {
			ev := event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindInfo,
				Name:  "attempt.candidate.end",
				Msg:   "candidate canceled",
				KVs: map[string]any{
					"path":      path,
					"candidate": cand.String(),
					"winner":    winner.String(),
					"reason":    "winner_selected",
				},
			}
			if cand == winner {
				ev.Kind = event.KindOK
				ev.Msg = "candidate ok"
				ev.KVs["reason"] = "reachable"
			}
			emit(ev)
		}

		evName := "attempt.v4.ok"
		evMsg := "ipv4 direct ok"
		if path == "direct_ipv6" {
			evName = "attempt.v6.ok"
			evMsg = "ipv6 direct ok"
		}
		emit(event.Event{
			Stage: event.StageAttempt,
			Kind:  event.KindOK,
			Name:  evName,
			Msg:   evMsg,
			KVs: map[string]any{
				"winner": winner.String(),
				"raddr":  raddr.String(),
				"ms":     time.Since(stepStart).Milliseconds(),
			},
		})
		return &AttemptResult{Path: path, Conn: conn, Remote: raddr}, nil
	}

	for _, cand := range candidates {
		emit(event.Event{
			Stage: event.StageAttempt,
			Kind:  event.KindInfo,
			Name:  "attempt.candidate.end",
			Msg:   "candidate timeout",
			Err:   err.Error(),
			KVs: map[string]any{
				"path":      path,
				"candidate": cand.String(),
				"reason":    "timeout",
			},
		})
	}
	if path == "direct_ipv6" {
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindInfo, Name: "attempt.v6.fail", Msg: "ipv6 direct failed", Err: err.Error()})
	} else {
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindInfo, Name: "attempt.v4.fail", Msg: "ipv4 direct failed", Err: err.Error()})
	}
	return nil, err
}

func attemptUDPPunching(ctx context.Context, sid string, key []byte, udp4Conn *net.UDPConn, resp *wire.NatHoleResp, cfg AttemptConfig, emit func(event.Event), punch PunchFunc, lastErr error) (*AttemptResult, error) {
	punchingPossible := resp.PunchingEnabled || len(resp.CandidateAddrs) > 0 || len(resp.AssistedAddrs) > 0
	if !punchingPossible {
		err := fmt.Errorf("punching disabled: %s", resp.PunchingError)
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindFail, Name: "attempt.punching.disabled", Msg: "punching disabled", Err: err.Error()})
		if lastErr != nil {
			return nil, fmt.Errorf("%w (udp fallback: %v)", err, lastErr)
		}
		return nil, err
	}
	if len(resp.CandidateAddrs) == 0 && len(resp.AssistedAddrs) == 0 {
		err := errors.New("punching enabled but both candidate_addrs and assisted_addrs empty")
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindFail, Name: "attempt.punching.invalid", Msg: "punching response invalid", Err: err.Error()})
		return nil, err
	}

	emit(event.Event{Stage: event.StageAttempt, Kind: event.KindStart, Name: "attempt.punching.start", Msg: "attempt punching"})
	newConn, raddr, err := punch(ctx, udp4Conn, resp, key)
	if err != nil {
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindFail, Name: "attempt.punching.fail", Msg: "punching failed", Err: err.Error()})
		return nil, err
	}

	emit(event.Event{
		Stage: event.StageAttempt,
		Kind:  event.KindOK,
		Name:  "attempt.punching.ok",
		Msg:   "punching ok",
		KVs: map[string]any{
			"raddr": raddr.String(),
		},
	})
	return &AttemptResult{Path: "punching_ipv4", Conn: newConn, Remote: raddr}, nil
}

func directHandshakeFanout(ctx context.Context, conn *net.UDPConn, sid string, key []byte, candidates []netip.AddrPort, sendCount int, sendInterval time.Duration) (*net.UDPAddr, netip.AddrPort, error) {
	if len(candidates) == 0 {
		return nil, netip.AddrPort{}, errors.New("no candidates")
	}

	successCh := make(chan *net.UDPAddr, 1)
	stopCh := make(chan struct{})
	stop := func() {
		select {
		case <-stopCh:
		default:
			close(stopCh)
		}
	}

	// Reader: MUST respond to request, and only accept response as "reachable".
	go func() {
		buf := make([]byte, 2048)
		for {
			_ = conn.SetReadDeadline(time.Now().Add(200 * time.Millisecond))
			n, raddr, err := conn.ReadFromUDP(buf)
			_ = conn.SetReadDeadline(time.Time{})
			if err != nil {
				select {
				case <-ctx.Done():
					return
				case <-stopCh:
					return
				default:
					continue
				}
			}

			var in wire.NatHoleSid
			if err := punching.DecodeMessageInto(buf[:n], key, &in); err != nil {
				continue
			}
			if in.Sid != sid {
				continue
			}

			if !in.Response {
				in.Response = true
				resp, err := punching.EncodeMessage(&in, key)
				if err == nil {
					_, _ = conn.WriteToUDP(resp, raddr)
				}
				// Keep waiting: a request only proves inbound reachability.
				continue
			}

			select {
			case successCh <- raddr:
				stop()
			default:
			}
			return
		}
	}()

	tx := punching.NewTransactionID()
	for _, ap := range candidates {
		ap := ap
		go func() {
			udpAddr := net.UDPAddrFromAddrPort(ap)
			req := &wire.NatHoleSid{
				TransactionID: tx,
				Sid:           sid,
				Response:      false,
			}
			payload, err := punching.EncodeMessage(req, key)
			if err != nil {
				return
			}
			for i := 0; i < sendCount; i++ {
				select {
				case <-ctx.Done():
					return
				case <-stopCh:
					return
				default:
				}
				_, _ = conn.WriteToUDP(payload, udpAddr)
				if i < sendCount-1 {
					select {
					case <-ctx.Done():
						return
					case <-stopCh:
						return
					case <-time.After(sendInterval):
					}
				}
			}
		}()
	}

	select {
	case raddr := <-successCh:
		winner := raddr.AddrPort()
		sendDirectHandshakeResponses(conn, sid, key, raddr, sendCount, sendInterval)
		return raddr, winner, nil
	case <-ctx.Done():
		return nil, netip.AddrPort{}, ctx.Err()
	}
}

func sendDirectHandshakeResponses(conn *net.UDPConn, sid string, key []byte, raddr *net.UDPAddr, sendCount int, sendInterval time.Duration) {
	if conn == nil || raddr == nil {
		return
	}
	if sendCount <= 0 {
		sendCount = 2
	}
	if sendInterval <= 0 {
		sendInterval = 50 * time.Millisecond
	}

	payload, err := punching.EncodeMessage(&wire.NatHoleSid{
		Sid:      sid,
		Response: true,
	}, key)
	if err != nil {
		return
	}

	for i := 0; i < sendCount; i++ {
		_, _ = conn.WriteToUDP(payload, raddr)
		if i+1 < sendCount {
			time.Sleep(sendInterval)
		}
	}
}
