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
	"github.com/miopunch/miopunch/internal/eventctx"
	"github.com/miopunch/miopunch/internal/punching"
	"github.com/miopunch/miopunch/internal/udpowner"
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

	// UDP4TraversalDemux / UDP6TraversalDemux, when set, provide the traversal
	// I/O boundary for UDP direct handshake and punching.
	//
	// If unset, Attempt will create a temporary demux reading from the
	// corresponding UDPConn and close it before returning.
	//
	// This allows callers to reuse a single UDP socket owner / demux (e.g. QUIC
	// Transport or a KCP owner) while supporting concurrent attempts.
	UDP4TraversalDemux *udpowner.TraversalDemux
	UDP6TraversalDemux *udpowner.TraversalDemux
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

type PunchFunc func(ctx context.Context, listenConn *net.UDPConn, demux *udpowner.TraversalDemux, resp *wire.NatHoleResp, key []byte) (*net.UDPConn, *net.UDPAddr, error)

func Attempt(ctx context.Context, sid string, key []byte, udp4Conn *net.UDPConn, udp6Conn *net.UDPConn, tcp4Listener *net.TCPListener, tcp6Listener *net.TCPListener, resp *wire.NatHoleResp, cfg AttemptConfig) (*AttemptResult, error) {
	return attemptWithPunch(ctx, sid, key, udp4Conn, udp6Conn, tcp4Listener, tcp6Listener, resp, cfg, punching.MakeHole)
}

func attemptWithPunch(ctx context.Context, sid string, key []byte, udp4Conn *net.UDPConn, udp6Conn *net.UDPConn, tcp4Listener *net.TCPListener, tcp6Listener *net.TCPListener, resp *wire.NatHoleResp, cfg AttemptConfig, punch PunchFunc) (*AttemptResult, error) {
	if resp == nil {
		return nil, errors.New("nil NatHoleResp")
	}

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

	udp4Demux := cfg.UDP4TraversalDemux
	udp6Demux := cfg.UDP6TraversalDemux
	closeUDP4Demux := func() {}
	closeUDP6Demux := func() {}
	defer func() {
		closeUDP4Demux()
		closeUDP6Demux()
	}()

	getUDP4Demux := func() (*udpowner.TraversalDemux, error) {
		if udp4Demux != nil {
			return udp4Demux, nil
		}
		if udp4Conn == nil {
			return nil, errors.New("udp4 conn is required")
		}
		d, err := udpowner.NewUDPTraversalDemux(udp4Conn, udpowner.DemuxConfig{Key: key})
		if err != nil {
			return nil, err
		}
		udp4Demux = d
		closeUDP4Demux = func() { _ = d.Close() }
		return d, nil
	}
	getUDP6Demux := func() (*udpowner.TraversalDemux, error) {
		if udp6Demux != nil {
			return udp6Demux, nil
		}
		if udp6Conn == nil {
			return nil, errors.New("udp6 conn is required")
		}
		d, err := udpowner.NewUDPTraversalDemux(udp6Conn, udpowner.DemuxConfig{Key: key})
		if err != nil {
			return nil, err
		}
		udp6Demux = d
		closeUDP6Demux = func() { _ = d.Close() }
		return d, nil
	}

	if allowUDP && cfg.P2PIPFamily == P2PIPFamilyV6 && udp6Conn == nil {
		return nil, errors.New("udp6 conn is required for p2p ip family v6")
	}
	if allowUDP && allowV4 && udp4Conn == nil {
		return nil, errors.New("udp4 conn is required")
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
		d, err := getUDP6Demux()
		if err != nil {
			return nil, err
		}
		res, err := attemptUDPDirect(ctx, sid, key, udp6Conn, d, peerUDPV6, cfg, emit, "direct_ipv6")
		if err == nil && res != nil {
			return res, nil
		}
		attemptErr = err
	}

	if allowUDP && allowV4 && udp4Conn != nil && len(peerUDPV4) > 0 {
		d, err := getUDP4Demux()
		if err != nil {
			return nil, err
		}
		res, err := attemptUDPDirect(ctx, sid, key, udp4Conn, d, peerUDPV4, cfg, emit, "direct_ipv4")
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
	d, err := getUDP4Demux()
	if err != nil {
		return nil, err
	}
	return attemptUDPPunching(ctx, sid, key, udp4Conn, d, resp, cfg, emit, punch, attemptErr)
}

func attemptUDPDirect(ctx context.Context, sid string, key []byte, conn *net.UDPConn, demux *udpowner.TraversalDemux, candidates []netip.AddrPort, cfg AttemptConfig, emit func(event.Event), path string) (*AttemptResult, error) {
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
	raddr, winner, err := directHandshakeFanout(subCtx, demux, sid, key, candidates, cfg.DirectSendCount, cfg.DirectSendInterval)
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

func attemptUDPPunching(ctx context.Context, sid string, key []byte, udp4Conn *net.UDPConn, demux *udpowner.TraversalDemux, resp *wire.NatHoleResp, cfg AttemptConfig, emit func(event.Event), punch PunchFunc, lastErr error) (*AttemptResult, error) {
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
	punchCtx := eventctx.WithEmitFunc(ctx, emit)
	newConn, raddr, err := punch(punchCtx, udp4Conn, demux, resp, key)
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

func directHandshakeFanout(ctx context.Context, demux *udpowner.TraversalDemux, sid string, key []byte, candidates []netip.AddrPort, sendCount int, sendInterval time.Duration) (*net.UDPAddr, netip.AddrPort, error) {
	if len(candidates) == 0 {
		return nil, netip.AddrPort{}, errors.New("no candidates")
	}
	if demux == nil {
		return nil, netip.AddrPort{}, errors.New("nil traversal demux")
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

	tx := punching.NewTransactionID()
	ep := demux.Open(tx, 8)
	defer ep.Close()

	// Reader: accept response as "reachable".
	go func() {
		buf := make([]byte, 2048)
		for {
			n, raddr, err := ep.Recv(ctx, buf)
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
				_ = ep.SendTo(ctx, payload, udpAddr, 0)
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
		sendDirectHandshakeResponses(ctx, ep, tx, sid, key, raddr, sendCount, sendInterval)
		return raddr, winner, nil
	case <-ctx.Done():
		return nil, netip.AddrPort{}, ctx.Err()
	}
}

func sendDirectHandshakeResponses(ctx context.Context, ep udpowner.TraversalEndpoint, transactionID string, sid string, key []byte, raddr *net.UDPAddr, sendCount int, sendInterval time.Duration) {
	if ep == nil || raddr == nil {
		return
	}
	if sendCount <= 0 {
		sendCount = 2
	}
	if sendInterval <= 0 {
		sendInterval = 50 * time.Millisecond
	}

	payload, err := punching.EncodeMessage(&wire.NatHoleSid{
		TransactionID: transactionID,
		Sid:           sid,
		Response:      true,
	}, key)
	if err != nil {
		return
	}

	for i := 0; i < sendCount; i++ {
		_ = ep.SendTo(ctx, payload, raddr, 0)
		if i+1 < sendCount {
			select {
			case <-ctx.Done():
				return
			case <-time.After(sendInterval):
			}
		}
	}
}
