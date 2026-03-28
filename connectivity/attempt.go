package connectivity

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"time"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/punching"
	"github.com/miopunch/miopunch/internal/wire"
)

type AttemptConfig struct {
	AttemptV6Timeout      time.Duration
	AttemptPortmapTimeout time.Duration

	DirectSendCount    int
	DirectSendInterval time.Duration

	Emitter *event.Emitter
}

type AttemptResult struct {
	Path   string
	Conn   *net.UDPConn
	Remote *net.UDPAddr
}

type PunchFunc func(ctx context.Context, listenConn *net.UDPConn, resp *wire.NatHoleResp, key []byte) (*net.UDPConn, *net.UDPAddr, error)

func Attempt(ctx context.Context, sid string, key []byte, udp4Conn *net.UDPConn, udp6Conn *net.UDPConn, resp *wire.NatHoleResp, cfg AttemptConfig) (*AttemptResult, error) {
	return attemptWithPunch(ctx, sid, key, udp4Conn, udp6Conn, resp, cfg, punching.MakeHole)
}

func attemptWithPunch(ctx context.Context, sid string, key []byte, udp4Conn *net.UDPConn, udp6Conn *net.UDPConn, resp *wire.NatHoleResp, cfg AttemptConfig, punch PunchFunc) (*AttemptResult, error) {
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

	if udp4Conn == nil {
		return nil, errors.New("udp4 conn is required")
	}
	if resp == nil {
		return nil, errors.New("nil NatHoleResp")
	}

	parsed := ParseDirectAddrPorts(resp.PeerDirectAddrs)
	if len(parsed.Invalid) > 0 {
		emit(event.Event{
			Stage: event.StageAttempt,
			Kind:  event.KindInfo,
			Name:  "attempt.peer_direct_addrs.invalid",
			Msg:   "invalid peer direct_addrs dropped",
			KVs: map[string]any{
				"count": len(parsed.Invalid),
			},
		})
	}

	peerV6, peerV4 := SplitAddrPortsByFamily(parsed.Addrs)
	emit(event.Event{
		Stage: event.StageAttempt,
		Kind:  event.KindStart,
		Name:  "attempt.start",
		Msg:   "attempt start",
		KVs: map[string]any{
			"peer_v6": len(peerV6),
			"peer_v4": len(peerV4),
		},
	})

	// 1) IPv6 direct
	if udp6Conn != nil && len(peerV6) > 0 {
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindStart, Name: "attempt.v6.start", Msg: "attempt ipv6 direct"})

		for _, cand := range peerV6 {
			emit(event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindStart,
				Name:  "attempt.candidate.begin",
				Msg:   "candidate begin",
				KVs: map[string]any{
					"path":      "direct_ipv6",
					"candidate": cand.String(),
				},
			})
		}

		stepStart := time.Now()
		subCtx, cancel := context.WithTimeout(ctx, cfg.AttemptV6Timeout)
		raddr, winner, err := directHandshakeFanout(subCtx, udp6Conn, sid, key, peerV6, cfg.DirectSendCount, cfg.DirectSendInterval)
		cancel()
		if err == nil {
			for _, cand := range peerV6 {
				ev := event.Event{
					Stage: event.StageAttempt,
					Kind:  event.KindInfo,
					Name:  "attempt.candidate.end",
					Msg:   "candidate canceled",
					KVs: map[string]any{
						"path":      "direct_ipv6",
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

			emit(event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindOK,
				Name:  "attempt.v6.ok",
				Msg:   "ipv6 direct ok",
				KVs: map[string]any{
					"winner": winner.String(),
					"raddr":  raddr.String(),
					"ms":     time.Since(stepStart).Milliseconds(),
				},
			})
			return &AttemptResult{Path: "direct_ipv6", Conn: udp6Conn, Remote: raddr}, nil
		}

		for _, cand := range peerV6 {
			emit(event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindInfo,
				Name:  "attempt.candidate.end",
				Msg:   "candidate timeout",
				Err:   err.Error(),
				KVs: map[string]any{
					"path":      "direct_ipv6",
					"candidate": cand.String(),
					"reason":    "timeout",
				},
			})
		}
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindInfo, Name: "attempt.v6.fail", Msg: "ipv6 direct failed", Err: err.Error()})
	}

	// 2) IPv4 direct (portmap candidates)
	if len(peerV4) > 0 {
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindStart, Name: "attempt.v4.start", Msg: "attempt ipv4 direct"})

		for _, cand := range peerV4 {
			emit(event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindStart,
				Name:  "attempt.candidate.begin",
				Msg:   "candidate begin",
				KVs: map[string]any{
					"path":      "direct_ipv4",
					"candidate": cand.String(),
				},
			})
		}

		stepStart := time.Now()
		subCtx, cancel := context.WithTimeout(ctx, cfg.AttemptPortmapTimeout)
		raddr, winner, err := directHandshakeFanout(subCtx, udp4Conn, sid, key, peerV4, cfg.DirectSendCount, cfg.DirectSendInterval)
		cancel()
		if err == nil {
			for _, cand := range peerV4 {
				ev := event.Event{
					Stage: event.StageAttempt,
					Kind:  event.KindInfo,
					Name:  "attempt.candidate.end",
					Msg:   "candidate canceled",
					KVs: map[string]any{
						"path":      "direct_ipv4",
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

			emit(event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindOK,
				Name:  "attempt.v4.ok",
				Msg:   "ipv4 direct ok",
				KVs: map[string]any{
					"winner": winner.String(),
					"raddr":  raddr.String(),
					"ms":     time.Since(stepStart).Milliseconds(),
				},
			})
			return &AttemptResult{Path: "direct_ipv4", Conn: udp4Conn, Remote: raddr}, nil
		}

		for _, cand := range peerV4 {
			emit(event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindInfo,
				Name:  "attempt.candidate.end",
				Msg:   "candidate timeout",
				Err:   err.Error(),
				KVs: map[string]any{
					"path":      "direct_ipv4",
					"candidate": cand.String(),
					"reason":    "timeout",
				},
			})
		}
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindInfo, Name: "attempt.v4.fail", Msg: "ipv4 direct failed", Err: err.Error()})
	}

	// 3) Punching fallback (P1 kernel)
	punchingPossible := resp.PunchingEnabled || len(resp.CandidateAddrs) > 0
	if !punchingPossible {
		err := fmt.Errorf("punching disabled: %s", resp.PunchingError)
		emit(event.Event{Stage: event.StageAttempt, Kind: event.KindFail, Name: "attempt.punching.disabled", Msg: "punching disabled", Err: err.Error()})
		return nil, err
	}
	if len(resp.CandidateAddrs) == 0 {
		err := errors.New("punching enabled but candidate_addrs empty")
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
