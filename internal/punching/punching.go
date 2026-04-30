// Copyright 2023 The frp Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

package punching

import (
	"context"
	"errors"
	"fmt"
	"math/rand/v2"
	"net"
	"os"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/fatedier/golib/pool"
	"golang.org/x/net/ipv4"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/eventctx"
	"github.com/miopunch/miopunch/internal/udpowner"
	"github.com/miopunch/miopunch/internal/wire"
	"github.com/miopunch/miopunch/internal/xlog"
	"github.com/miopunch/miopunch/nat"
)

var (
	// mode 0: simple detect mode, usually for both EasyNAT or HardNAT & EasyNAT(Public Network)
	// a. receiver sends detect message with low TTL
	// b. sender sends normal detect message to receiver
	// c. receiver receives detect message and sends back a message to sender
	//
	// mode 1: For HardNAT & EasyNAT, send detect messages to multiple guessed ports.
	// Usually applicable to scenarios where port changes are regular.
	// Most of the steps are the same as mode 0, but EasyNAT is fixed as the receiver and will send detect messages
	// with low TTL to multiple guessed ports of the sender.
	//
	// mode 2: For HardNAT & EasyNAT, ports changes are not regular.
	// a. HardNAT machine will listen on multiple ports and send detect messages with low TTL to EasyNAT machine
	// b. EasyNAT machine will send detect messages to random ports of HardNAT machine.
	//
	// mode 3: For HardNAT & HardNAT, both changes in the ports are regular.
	// Most of the steps are the same as mode 1, but the sender also needs to send detect messages to multiple guessed
	// ports of the receiver.
	//
	// mode 4: For HardNAT & HardNAT, one of the changes in the ports is regular.
	// Regular port changes are usually on the sender side.
	// a. Receiver listens on multiple ports and sends detect messages with low TTL to the sender's guessed range ports.
	// b. Sender sends detect messages to random ports of the receiver.
	SupportedModes = []int{DetectMode0, DetectMode1, DetectMode2, DetectMode3, DetectMode4}
	SupportedRoles = []string{DetectRoleSender, DetectRoleReceiver}

	DetectMode0        = 0
	DetectMode1        = 1
	DetectMode2        = 2
	DetectMode3        = 3
	DetectMode4        = 4
	DetectRoleSender   = "sender"
	DetectRoleReceiver = "receiver"
)

// PrepareOptions defines options for NAT traversal preparation
type PrepareOptions struct {
	// DisableAssistedAddrs disables the use of local network interfaces
	// for assisted connections during NAT traversal
	DisableAssistedAddrs bool
}

type PrepareResult struct {
	Addrs         []string
	AssistedAddrs []string
	ListenConn    *net.UDPConn
	NatType       string
	Behavior      string
}

// PreCheck is used to check if the proxy is ready for penetration.
// Call this function before calling Prepare to avoid unnecessary preparation work.
func PreCheck(
	ctx context.Context, transporter wire.MessageTransporter,
	proxyName string, timeout time.Duration,
) error {
	timeoutCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	var natHoleRespMsg *wire.NatHoleResp
	transactionID := NewTransactionID()
	m, err := transporter.Do(timeoutCtx, &wire.NatHoleVisitor{
		TransactionID: transactionID,
		ProxyName:     proxyName,
		PreCheck:      true,
	}, transactionID, wire.TypeNameNatHoleResp)
	if err != nil {
		return fmt.Errorf("get natHoleRespMsg error: %v", err)
	}
	mm, ok := m.(*wire.NatHoleResp)
	if !ok {
		return fmt.Errorf("get natHoleRespMsg error: invalid message type")
	}
	natHoleRespMsg = mm

	if natHoleRespMsg.Error != "" {
		return fmt.Errorf("%s", natHoleRespMsg.Error)
	}
	return nil
}

// Prepare is used to do some preparation work before penetration.
func Prepare(stunServers []string, opts PrepareOptions) (*PrepareResult, error) {
	// discover for Nat type
	addrs, localAddr, err := nat.Discover(stunServers, "")
	if err != nil {
		return nil, fmt.Errorf("discover error: %v", err)
	}
	if len(addrs) < 2 {
		return nil, fmt.Errorf("discover error: not enough addresses")
	}

	localIPs, _ := nat.ListLocalIPsForNatHole(10)
	natFeature, err := nat.ClassifyNATFeature(addrs, localIPs)
	if err != nil {
		return nil, fmt.Errorf("classify nat feature error: %v", err)
	}

	laddr, err := net.ResolveUDPAddr("udp4", localAddr.String())
	if err != nil {
		return nil, fmt.Errorf("resolve local udp addr error: %v", err)
	}
	listenConn, err := net.ListenUDP("udp4", laddr)
	if err != nil {
		return nil, fmt.Errorf("listen local udp addr error: %v", err)
	}

	// Apply NAT traversal options
	var assistedAddrs []string
	if !opts.DisableAssistedAddrs {
		assistedAddrs = make([]string, 0, len(localIPs))
		for _, ip := range localIPs {
			assistedAddrs = append(assistedAddrs, net.JoinHostPort(ip, strconv.Itoa(laddr.Port)))
		}
	}
	return &PrepareResult{
		Addrs:         addrs,
		AssistedAddrs: assistedAddrs,
		ListenConn:    listenConn,
		NatType:       natFeature.NatType,
		Behavior:      natFeature.Behavior,
	}, nil
}

// ExchangeInfo is used to exchange information between client and visitor.
// 1. Send input message to server by msgTransporter.
// 2. Server will gather information from client and visitor and analyze it. Then send back a NatHoleResp message to them to tell them how to do next.
// 3. Receive NatHoleResp message from server.
func ExchangeInfo(
	ctx context.Context, transporter wire.MessageTransporter,
	laneKey string, m wire.Message, timeout time.Duration,
) (*wire.NatHoleResp, error) {
	timeoutCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	var natHoleRespMsg *wire.NatHoleResp
	m, err := transporter.Do(timeoutCtx, m, laneKey, wire.TypeNameNatHoleResp)
	if err != nil {
		return nil, fmt.Errorf("get natHoleRespMsg error: %v", err)
	}
	mm, ok := m.(*wire.NatHoleResp)
	if !ok {
		return nil, fmt.Errorf("get natHoleRespMsg error: invalid message type")
	}
	natHoleRespMsg = mm

	if natHoleRespMsg.Error != "" {
		return nil, fmt.Errorf("natHoleRespMsg get error info: %s", natHoleRespMsg.Error)
	}

	// Backward-compatible punching gate:
	// - Old control plane had no punching_enabled field; presence of candidate_addrs implies punching is possible.
	// - New control plane may disable punching (e.g. STUN missing) while still exchanging peer_direct_addrs.
	punchingPossible := natHoleRespMsg.PunchingEnabled || len(natHoleRespMsg.CandidateAddrs) > 0 || len(natHoleRespMsg.AssistedAddrs) > 0
	if natHoleRespMsg.PunchingEnabled == false && (len(natHoleRespMsg.CandidateAddrs) > 0 || len(natHoleRespMsg.AssistedAddrs) > 0) {
		natHoleRespMsg.PunchingEnabled = true
		natHoleRespMsg.PunchingError = ""
	}

	if punchingPossible && len(natHoleRespMsg.CandidateAddrs) == 0 && len(natHoleRespMsg.AssistedAddrs) == 0 {
		return nil, fmt.Errorf("natHoleRespMsg has no candidate addresses and no assisted addresses while punching enabled")
	}
	return natHoleRespMsg, nil
}

const (
	udpPunchProbeInterval         = 200 * time.Millisecond
	udpPunchReadPollInterval      = 200 * time.Millisecond
	udpPunchResponseBurstCount    = 3
	udpPunchResponseBurstInterval = 50 * time.Millisecond
)

type udpConnWriter struct {
	conn *net.UDPConn
	mu   sync.Mutex

	ttlDegradedOnce sync.Once
}

func newUDPConnWriter(conn *net.UDPConn) *udpConnWriter {
	if conn == nil {
		return nil
	}
	return &udpConnWriter{
		conn: conn,
	}
}

var (
	defaultIPv4TTLOnce   sync.Once
	defaultIPv4TTLCached int
)

func defaultIPv4TTL() int {
	defaultIPv4TTLOnce.Do(func() {
		const fallback = 64

		b, err := os.ReadFile("/proc/sys/net/ipv4/ip_default_ttl")
		if err != nil {
			defaultIPv4TTLCached = fallback
			return
		}
		v, err := strconv.Atoi(strings.TrimSpace(string(b)))
		if err != nil || v <= 0 || v > 255 {
			defaultIPv4TTLCached = fallback
			return
		}
		defaultIPv4TTLCached = v
	})
	if defaultIPv4TTLCached <= 0 {
		return 64
	}
	return defaultIPv4TTLCached
}

func (w *udpConnWriter) WriteToUDP(ctx context.Context, buf []byte, raddr *net.UDPAddr, ttl int) error {
	if w == nil || w.conn == nil {
		return errors.New("nil udp conn writer")
	}
	if raddr == nil {
		return errors.New("nil udp remote addr")
	}
	w.mu.Lock()
	defer w.mu.Unlock()

	from := ""
	if w.conn.LocalAddr() != nil {
		from = w.conn.LocalAddr().String()
	}

	var (
		origTTL int
		origOK  bool

		getErr     error
		setErr     error
		restoreErr error
	)

	var uConn *ipv4.Conn
	if ttl > 0 {
		uConn = ipv4.NewConn(w.conn)
		origTTL, getErr = uConn.TTL()
		if getErr == nil {
			origOK = true
		}
		setErr = uConn.SetTTL(ttl)
	}

	_, err := w.conn.WriteToUDP(buf, raddr)

	if ttl > 0 && setErr == nil && uConn != nil {
		restoreTo := defaultIPv4TTL()
		if origOK {
			restoreTo = origTTL
		}
		restoreErr = uConn.SetTTL(restoreTo)
	}

	if ttl > 0 && (getErr != nil || setErr != nil || restoreErr != nil) {
		w.ttlDegradedOnce.Do(func() {
			kvs := map[string]any{
				"requested_ttl": ttl,
				"from":          from,
				"to":            raddr.String(),
			}
			if getErr != nil {
				kvs["get_err"] = getErr.Error()
			}
			if setErr != nil {
				kvs["set_err"] = setErr.Error()
			}
			if restoreErr != nil {
				kvs["restore_err"] = restoreErr.Error()
			}
			eventctx.Emit(ctx, event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindInfo,
				Name:  "attempt.punching.probe.ttl.degraded",
				Msg:   "punching ttl degraded; proceeding with default ttl",
				KVs:   kvs,
			})
		})
	}

	return err
}

type udpPhasePlan struct {
	Mode int
	Role string
	TTL  int

	SendDelay     time.Duration
	TotalBudget   time.Duration
	ProbeInterval time.Duration

	DetectAddrs []string

	CandidateAddrs []string
	CandidatePorts []wire.PortsRange

	SendRandomPorts   int
	ListenRandomPorts int
}

func buildUDPPhasePlan(m *wire.NatHoleResp) (udpPhasePlan, error) {
	if m == nil {
		return udpPhasePlan{}, errors.New("nil NatHoleResp")
	}

	role := m.DetectBehavior.Role
	if role != DetectRoleSender && role != DetectRoleReceiver {
		return udpPhasePlan{}, fmt.Errorf("invalid detect role: %q", role)
	}

	sendDelayMs := max(m.DetectBehavior.SendDelayMs, 0)
	readTimeoutMs := m.DetectBehavior.ReadTimeoutMs
	if readTimeoutMs <= 0 {
		readTimeoutMs = 5000
	}
	totalBudget := time.Duration(sendDelayMs+readTimeoutMs) * time.Millisecond
	if totalBudget <= 0 {
		totalBudget = 5 * time.Second
	}

	detectAddrs := make([]string, 0, len(m.CandidateAddrs)+len(m.AssistedAddrs))
	if role == DetectRoleSender {
		detectAddrs = append(detectAddrs, m.AssistedAddrs...)
		detectAddrs = append(detectAddrs, m.CandidateAddrs...)
	} else {
		// Preserve previous behavior: when the receiver is probing a candidate port
		// range, it does not also send to the explicit candidate addrs.
		if len(m.DetectBehavior.CandidatePorts) == 0 {
			detectAddrs = append(detectAddrs, m.CandidateAddrs...)
		}
	}
	detectAddrs = slices.Compact(detectAddrs)

	return udpPhasePlan{
		Mode: m.DetectBehavior.Mode,
		Role: role,
		TTL:  m.DetectBehavior.TTL,

		SendDelay:     time.Duration(sendDelayMs) * time.Millisecond,
		TotalBudget:   totalBudget,
		ProbeInterval: udpPunchProbeInterval,

		DetectAddrs: detectAddrs,

		CandidateAddrs:    slices.Compact(slices.Clone(m.CandidateAddrs)),
		CandidatePorts:    slices.Clone(m.DetectBehavior.CandidatePorts),
		SendRandomPorts:   m.DetectBehavior.SendRandomPorts,
		ListenRandomPorts: m.DetectBehavior.ListenRandomPorts,
	}, nil
}

func sleepWithContext(ctx context.Context, d time.Duration) bool {
	if d <= 0 {
		return true
	}
	timer := time.NewTimer(d)
	defer timer.Stop()
	select {
	case <-timer.C:
		return true
	case <-ctx.Done():
		return false
	}
}

// MakeHole is used to make a NAT hole between client and visitor.
func MakeHole(ctx context.Context, listenConn *net.UDPConn, demux *udpowner.TraversalDemux, m *wire.NatHoleResp, key []byte) (*net.UDPConn, *net.UDPAddr, error) {
	xl := xlog.FromContextSafe(ctx)

	if listenConn == nil {
		return nil, nil, errors.New("listen udp conn is required")
	}
	if demux == nil {
		return nil, nil, errors.New("nil traversal demux")
	}
	if m == nil {
		return nil, nil, errors.New("nil NatHoleResp")
	}
	if strings.TrimSpace(m.Sid) == "" {
		return nil, nil, errors.New("missing sid in NatHoleResp")
	}

	plan, err := buildUDPPhasePlan(m)
	if err != nil {
		return nil, nil, err
	}

	start := time.Now()
	subCtx, cancel := context.WithTimeout(ctx, plan.TotalBudget)
	defer cancel()

	transactionID := traversalTransactionID(m.Sid, m.TransactionID)
	ep := demux.Open(transactionID, 32)
	defer ep.Close()

	eventctx.Emit(ctx, event.Event{
		Stage: event.StageAttempt,
		Kind:  event.KindInfo,
		Name:  "attempt.punching.recv.start",
		Msg:   "punching receive loop start",
		KVs: map[string]any{
			"mode":                plan.Mode,
			"role":                plan.Role,
			"ttl":                 plan.TTL,
			"listen_ports":        1,
			"listen_random_ports": 0,
		},
	})

	type detectResult struct {
		raddr *net.UDPAddr
		kind  string // request | response
	}

	resultCh := make(chan detectResult, 1)
	var wg sync.WaitGroup
	firstMsgOnce := sync.Once{}
	firstSendOnce := sync.Once{}

	recvLoop := func() {
		defer wg.Done()
		for {
			buf := pool.GetBuf(2048)
			n, raddr, err := ep.Recv(subCtx, buf)
			if err != nil {
				pool.PutBuf(buf)
				return
			}

			var msg wire.NatHoleSid
			if err := DecodeMessageInto(buf[:n], key, &msg); err != nil {
				pool.PutBuf(buf)
				continue
			}
			pool.PutBuf(buf)

			if msg.Sid != m.Sid {
				continue
			}

			msgKind := "response"
			if !msg.Response {
				msgKind = "request"
			}

			firstMsgOnce.Do(func() {
				eventctx.Emit(ctx, event.Event{
					Stage: event.StageAttempt,
					Kind:  event.KindInfo,
					Name:  "attempt.punching.msg.first",
					Msg:   "first sid message observed",
					KVs: map[string]any{
						"kind":  msgKind,
						"raddr": raddr.String(),
					},
				})
			})

			if !msg.Response {
				// Only respond to request messages if we are a receiver.
				if plan.Role == DetectRoleSender {
					continue
				}
				msg.Response = true
				out, err := EncodeMessage(&msg, key)
				if err != nil {
					continue
				}
				for i := 0; i < udpPunchResponseBurstCount; i++ {
					_ = ep.SendTo(subCtx, out, raddr, 0)
					if i+1 < udpPunchResponseBurstCount {
						if !sleepWithContext(subCtx, udpPunchResponseBurstInterval) {
							break
						}
					}
				}
			}

			select {
			case resultCh <- detectResult{raddr: raddr, kind: msgKind}:
			default:
			}
			return
		}
	}

	wg.Add(1)
	go recvLoop()

	if plan.SendDelay > 0 {
		timer := time.NewTimer(plan.SendDelay)
		select {
		case res := <-resultCh:
			cancel()
			timer.Stop()
			wg.Wait()
			eventctx.Emit(ctx, event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindOK,
				Name:  "attempt.punching.winner",
				Msg:   "punching winner selected",
				KVs: map[string]any{
					"kind":       res.kind,
					"raddr":      res.raddr.String(),
					"elapsed_ms": time.Since(start).Milliseconds(),
				},
			})
			return listenConn, res.raddr, nil
		case <-timer.C:
		case <-subCtx.Done():
			timer.Stop()
			cancel()
			wg.Wait()
			doneErr := subCtx.Err()
			evName := "attempt.punching.timeout"
			evMsg := "punching timeout before probe start"
			if errors.Is(doneErr, context.Canceled) {
				evName = "attempt.punching.canceled"
				evMsg = "punching canceled before probe start"
			}
			eventctx.Emit(ctx, event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindInfo,
				Name:  evName,
				Msg:   evMsg,
				Err:   doneErr.Error(),
			})
			return nil, nil, fmt.Errorf("wait detect message error: %w", doneErr)
		}
	}

	eventctx.Emit(ctx, event.Event{
		Stage: event.StageAttempt,
		Kind:  event.KindInfo,
		Name:  "attempt.punching.probe.start",
		Msg:   "punching probe loop start",
		KVs: map[string]any{
			"mode":                plan.Mode,
			"role":                plan.Role,
			"ttl":                 plan.TTL,
			"send_delay_ms":       int(plan.SendDelay.Milliseconds()),
			"total_budget_ms":     int(plan.TotalBudget.Milliseconds()),
			"probe_interval_ms":   int(plan.ProbeInterval.Milliseconds()),
			"detect_addrs":        len(plan.DetectAddrs),
			"candidate_ports":     len(plan.CandidatePorts),
			"send_random_ports":   plan.SendRandomPorts,
			"listen_random_ports": plan.ListenRandomPorts,
		},
	})

	// Probe loop: always send at least one burst immediately after send delay.
	didStartRandom := false
	sendProbeRound := func(first bool) {
		if first {
			eventctx.Emit(ctx, event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindInfo,
				Name:  "attempt.punching.probe.first",
				Msg:   "punching first probe burst",
			})
		}

		sendErrs := 0
		firstSendErr := ""
		firstSendAddr := ""
		for _, detectAddr := range plan.DetectAddrs {
			if first {
				firstSendOnce.Do(func() {
					from := ""
					if listenConn != nil && listenConn.LocalAddr() != nil {
						from = listenConn.LocalAddr().String()
					}
					eventctx.Emit(ctx, event.Event{
						Stage: event.StageAttempt,
						Kind:  event.KindInfo,
						Name:  "attempt.punching.probe.send.first",
						Msg:   "punching first probe send attempt",
						KVs: map[string]any{
							"from": from,
							"to":   detectAddr,
							"ttl":  plan.TTL,
						},
					})
				})
			}
			if err := sendSidMessage(subCtx, ep, m.Sid, transactionID, detectAddr, key, plan.TTL); err != nil {
				sendErrs++
				if firstSendErr == "" {
					firstSendErr = err.Error()
					firstSendAddr = detectAddr
				}
				from := ""
				if listenConn != nil && listenConn.LocalAddr() != nil {
					from = listenConn.LocalAddr().String()
				}
				xl.Tracef("send sid message from %s to %s error: %v", from, detectAddr, err)
			}
		}
		if first && sendErrs > 0 {
			eventctx.Emit(ctx, event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindInfo,
				Name:  "attempt.punching.probe.send.error",
				Msg:   "punching probe send errors",
				KVs: map[string]any{
					"count":       sendErrs,
					"first_addr":  firstSendAddr,
					"first_error": firstSendErr,
				},
			})
		}

		if first && len(plan.CandidatePorts) > 0 {
			sendSidMessageToRangePorts(subCtx, plan.CandidateAddrs, plan.CandidatePorts, func(addr string) error {
				return sendSidMessage(subCtx, ep, m.Sid, transactionID, addr, key, plan.TTL)
			})
		}

		if first && plan.SendRandomPorts > 0 && !didStartRandom {
			didStartRandom = true
			wg.Add(1)
			go func() {
				defer wg.Done()
				sendSidMessageToRandomPorts(subCtx, plan.CandidateAddrs, plan.SendRandomPorts, func(addr string) error {
					return sendSidMessage(subCtx, ep, m.Sid, transactionID, addr, key, plan.TTL)
				})
			}()
		}
	}

	sendProbeRound(true)

	ticker := time.NewTicker(plan.ProbeInterval)
	defer ticker.Stop()
	for {
		select {
		case res := <-resultCh:
			cancel()
			wg.Wait()
			eventctx.Emit(ctx, event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindOK,
				Name:  "attempt.punching.winner",
				Msg:   "punching winner selected",
				KVs: map[string]any{
					"kind":       res.kind,
					"raddr":      res.raddr.String(),
					"elapsed_ms": time.Since(start).Milliseconds(),
				},
			})
			return listenConn, res.raddr, nil
		case <-ticker.C:
			sendProbeRound(false)
		case <-subCtx.Done():
			cancel()
			wg.Wait()
			doneErr := subCtx.Err()
			evName := "attempt.punching.timeout"
			evMsg := "punching timeout"
			if errors.Is(doneErr, context.Canceled) {
				evName = "attempt.punching.canceled"
				evMsg = "punching canceled"
			}
			eventctx.Emit(ctx, event.Event{
				Stage: event.StageAttempt,
				Kind:  event.KindInfo,
				Name:  evName,
				Msg:   evMsg,
				Err:   doneErr.Error(),
				KVs: map[string]any{
					"elapsed_ms": time.Since(start).Milliseconds(),
				},
			})
			return nil, nil, fmt.Errorf("wait detect message error: %w", doneErr)
		}
	}
}

func closeNonWinnerUDPConns(conns []*net.UDPConn, winner *net.UDPConn) error {
	for _, conn := range conns {
		if conn == nil || conn == winner {
			continue
		}
		_ = conn.Close()
	}
	return nil
}

func sendSidMessage(
	ctx context.Context, ep udpowner.TraversalEndpoint,
	sid string, transactionID string, addr string, key []byte, ttl int,
) error {
	xl := xlog.FromContextSafe(ctx)
	if ep == nil {
		return errors.New("nil traversal endpoint")
	}
	ttlStr := ""
	if ttl > 0 {
		ttlStr = fmt.Sprintf(" with ttl %d", ttl)
	}
	xl.Tracef("send sid message to %s%s", addr, ttlStr)
	raddr, err := net.ResolveUDPAddr("udp4", addr)
	if err != nil {
		return err
	}
	if transactionID == "" {
		transactionID = NewTransactionID()
	}
	m := &wire.NatHoleSid{
		TransactionID: transactionID,
		Sid:           sid,
		Response:      false,
		Nonce:         strings.Repeat("0", rand.IntN(20)),
	}
	buf, err := EncodeMessage(m, key)
	if err != nil {
		return err
	}
	return ep.SendTo(ctx, buf, raddr, ttl)
}

func traversalTransactionID(sid string, fallback string) string {
	if sid = strings.TrimSpace(sid); sid != "" {
		return sid
	}
	if fallback = strings.TrimSpace(fallback); fallback != "" {
		return fallback
	}
	return NewTransactionID()
}

func sendSidMessageToRangePorts(
	ctx context.Context, addrs []string, ports []wire.PortsRange,
	sendFunc func(string) error,
) {
	xl := xlog.FromContextSafe(ctx)
	for _, ip := range slices.Compact(parseIPs(addrs)) {
		select {
		case <-ctx.Done():
			return
		default:
		}
		for _, portsRange := range ports {
			for i := portsRange.From; i <= portsRange.To; i++ {
				select {
				case <-ctx.Done():
					return
				default:
				}
				detectAddr := net.JoinHostPort(ip, strconv.Itoa(i))
				if err := sendFunc(detectAddr); err != nil {
					xl.Tracef("send sid message to %s error: %v", detectAddr, err)
				}
				if !sleepWithContext(ctx, 2*time.Millisecond) {
					return
				}
			}
		}
	}
}

func sendSidMessageToRandomPorts(
	ctx context.Context, addrs []string, count int,
	sendFunc func(string) error,
) {
	xl := xlog.FromContextSafe(ctx)
	used := make(map[int]struct{})
	getUnusedPort := func() int {
		for range 10 {
			port := rand.IntN(65535-1024) + 1024
			if _, ok := used[port]; !ok {
				used[port] = struct{}{}
				return port
			}
		}
		return 0
	}

	for range count {
		select {
		case <-ctx.Done():
			return
		default:
		}

		port := getUnusedPort()
		if port == 0 {
			continue
		}

		for _, ip := range slices.Compact(parseIPs(addrs)) {
			detectAddr := net.JoinHostPort(ip, strconv.Itoa(port))
			if err := sendFunc(detectAddr); err != nil {
				xl.Tracef("send sid message to %s error: %v", detectAddr, err)
			}
			if !sleepWithContext(ctx, 15*time.Millisecond) {
				return
			}
		}
	}
}

func parseIPs(addrs []string) []string {
	var ips []string
	for _, addr := range addrs {
		if ip, _, err := net.SplitHostPort(addr); err == nil {
			ips = append(ips, ip)
		}
	}
	return ips
}
