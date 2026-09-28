package punch

import (
	"context"
	"errors"
	"fmt"
	"net"
	"strings"
	"time"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/eventctx"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/punching"
	"github.com/miopunch/miopunch/internal/udpowner"
	legacywire "github.com/miopunch/miopunch/internal/wire"
)

var errDirectIPv4NotApplicable = errors.New("direct_ipv4 not applicable")

func attemptDirectIPv4(
	ctx context.Context,
	demux *udpowner.TraversalDemux,
	plan pairPlan,
	key []byte,
) (*net.UDPAddr, error) {
	_, remoteAddr, ok, err := directIPv4CandidateAddrs(plan)
	if err != nil {
		return nil, err
	}
	if !ok {
		return nil, errDirectIPv4NotApplicable
	}
	if demux == nil {
		return nil, errors.New("nil traversal demux")
	}

	attemptCtx, cancel := context.WithTimeout(ctx, defaultDirectTimeout)
	defer cancel()

	eventctx.Emit(attemptCtx, event.Event{
		Stage: event.StageAttempt,
		Kind:  event.KindStart,
		Name:  "attempt.direct_ipv4.start",
		Msg:   "attempt direct ipv4",
		KVs: map[string]any{
			"local_candidate":  plan.local.Addr,
			"remote_candidate": plan.remote.Addr,
			"timeout_ms":       defaultDirectTimeout.Milliseconds(),
		},
	})
	logutil.Tracef(
		"pocv1 direct ipv4 start: sid=%s local_candidate=%s remote_candidate=%s timeout_ms=%d",
		plan.sid,
		plan.local.Addr,
		plan.remote.Addr,
		defaultDirectTimeout.Milliseconds(),
	)

	raddr, err := directIPv4Handshake(
		attemptCtx,
		demux,
		plan.sid,
		key,
		remoteAddr,
		defaultDirectSendCount,
		defaultDirectSendInterval,
	)
	if err != nil {
		eventctx.Emit(ctx, event.Event{
			Stage: event.StageAttempt,
			Kind:  event.KindInfo,
			Name:  "attempt.direct_ipv4.fail",
			Msg:   "direct ipv4 failed",
			Err:   err.Error(),
			KVs: map[string]any{
				"remote_candidate": plan.remote.Addr,
			},
		})
		logutil.Tracef(
			"pocv1 direct ipv4 failed: sid=%s local_candidate=%s remote_candidate=%s err=%v",
			plan.sid,
			plan.local.Addr,
			plan.remote.Addr,
			err,
		)
		return nil, err
	}

	eventctx.Emit(ctx, event.Event{
		Stage: event.StageAttempt,
		Kind:  event.KindOK,
		Name:  "attempt.direct_ipv4.ok",
		Msg:   "direct ipv4 ok",
		KVs: map[string]any{
			"raddr": raddr.String(),
		},
	})
	logutil.Tracef(
		"pocv1 direct ipv4 ok: sid=%s local_candidate=%s remote_candidate=%s remote_udp=%s",
		plan.sid,
		plan.local.Addr,
		plan.remote.Addr,
		raddr.String(),
	)
	return raddr, nil
}

func directIPv4CandidateAddrs(plan pairPlan) (*net.UDPAddr, *net.UDPAddr, bool, error) {
	if plan.local.Kind != CandidateKindHost || plan.remote.Kind != CandidateKindHost {
		return nil, nil, false, nil
	}

	localAddr, err := net.ResolveUDPAddr("udp4", plan.local.Addr)
	if err != nil {
		return nil, nil, false, fmt.Errorf("resolve local direct ipv4 candidate: %w", err)
	}
	remoteAddr, err := net.ResolveUDPAddr("udp4", plan.remote.Addr)
	if err != nil {
		return nil, nil, false, fmt.Errorf("resolve remote direct ipv4 candidate: %w", err)
	}
	if localAddr == nil || remoteAddr == nil || localAddr.IP == nil || remoteAddr.IP == nil {
		return nil, nil, false, nil
	}
	if localAddr.IP.To4() == nil || remoteAddr.IP.To4() == nil {
		return nil, nil, false, nil
	}
	if localAddr.Port == 0 || remoteAddr.Port == 0 {
		return nil, nil, false, nil
	}
	if localAddr.IP.Equal(remoteAddr.IP) {
		return nil, nil, false, nil
	}
	return localAddr, remoteAddr, true, nil
}

func directIPv4Handshake(
	ctx context.Context,
	demux *udpowner.TraversalDemux,
	sid string,
	key []byte,
	remoteAddr *net.UDPAddr,
	sendCount int,
	sendInterval time.Duration,
) (*net.UDPAddr, error) {
	sid = strings.TrimSpace(sid)
	if sid == "" {
		return nil, errors.New("missing direct ipv4 sid")
	}
	if remoteAddr == nil {
		return nil, errors.New("nil direct ipv4 remote addr")
	}
	if sendCount <= 0 {
		sendCount = defaultDirectSendCount
	}
	if sendInterval <= 0 {
		sendInterval = defaultDirectSendInterval
	}

	ep := demux.Open(sid, 8)
	defer ep.Close()

	successCh := make(chan *net.UDPAddr, 1)
	stopCh := make(chan struct{})
	stop := func() {
		select {
		case <-stopCh:
		default:
			close(stopCh)
		}
	}

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
					logutil.Tracef("pocv1 direct ipv4 recv error: sid=%s err=%v", sid, err)
					continue
				}
			}

			var msg legacywire.NatHoleSid
			if err := punching.DecodeMessageInto(buf[:n], key, &msg); err != nil {
				logutil.Tracef("pocv1 direct ipv4 decode failed: sid=%s remote=%s err=%v", sid, raddr, err)
				continue
			}
			if msg.Sid != sid {
				logutil.Tracef("pocv1 direct ipv4 sid mismatch: want=%s got=%s remote=%s", sid, msg.Sid, raddr)
				continue
			}

			if !msg.Response {
				sendDirectIPv4Responses(ctx, ep, sid, key, raddr, sendCount, sendInterval)
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

	req, err := punching.EncodeMessage(&legacywire.NatHoleSid{
		TransactionID: sid,
		Sid:           sid,
		Response:      false,
	}, key)
	if err != nil {
		stop()
		return nil, fmt.Errorf("encode direct ipv4 request: %w", err)
	}

sendLoop:
	for i := 0; i < sendCount; i++ {
		select {
		case <-ctx.Done():
			stop()
			return nil, ctx.Err()
		case <-stopCh:
			break sendLoop
		default:
		}
		if err := ep.SendTo(ctx, req, remoteAddr, 0); err != nil {
			logutil.Tracef("pocv1 direct ipv4 send failed: sid=%s remote=%s err=%v", sid, remoteAddr, err)
		} else if i == 0 {
			logutil.Tracef("pocv1 direct ipv4 first request sent: sid=%s remote=%s", sid, remoteAddr)
		}
		if i+1 < sendCount {
			select {
			case <-ctx.Done():
				stop()
				return nil, ctx.Err()
			case <-stopCh:
				break sendLoop
			case <-time.After(sendInterval):
			}
		}
	}

	select {
	case raddr := <-successCh:
		sendDirectIPv4Responses(ctx, ep, sid, key, raddr, sendCount, sendInterval)
		return raddr, nil
	case <-ctx.Done():
		stop()
		return nil, ctx.Err()
	}
}

func sendDirectIPv4Responses(
	ctx context.Context,
	ep udpowner.TraversalEndpoint,
	sid string,
	key []byte,
	raddr *net.UDPAddr,
	sendCount int,
	sendInterval time.Duration,
) {
	if ep == nil || raddr == nil {
		return
	}
	if sendCount <= 0 {
		sendCount = defaultDirectSendCount
	}
	if sendInterval <= 0 {
		sendInterval = defaultDirectSendInterval
	}

	payload, err := punching.EncodeMessage(&legacywire.NatHoleSid{
		TransactionID: sid,
		Sid:           sid,
		Response:      true,
	}, key)
	if err != nil {
		logutil.Tracef("pocv1 direct ipv4 response encode failed: sid=%s err=%v", sid, err)
		return
	}
	for i := 0; i < sendCount; i++ {
		if err := ep.SendTo(ctx, payload, raddr, 0); err != nil {
			logutil.Tracef("pocv1 direct ipv4 response send failed: sid=%s remote=%s err=%v", sid, raddr, err)
			return
		}
		if i == 0 {
			logutil.Tracef("pocv1 direct ipv4 first response sent: sid=%s remote=%s", sid, raddr)
		}
		if i+1 < sendCount {
			select {
			case <-ctx.Done():
				return
			case <-time.After(sendInterval):
			}
		}
	}
}
