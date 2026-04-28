// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

package mqtt

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"
	"sync"
	"time"

	mqttclient "github.com/256dpi/gomqtt/client"
	"github.com/256dpi/gomqtt/client/future"
	"github.com/256dpi/gomqtt/packet"

	"github.com/miopunch/miopunch/internal/wire"
)

type Role string

const (
	RoleClient  Role = "client"
	RoleVisitor Role = "visitor"
)

var ErrTooManyAttempts = errors.New("too many concurrent mqtt attempts")

type Config struct {
	BrokerURL   string
	TopicPrefix string

	SID  string
	Role Role

	HelloInterval   time.Duration
	HelloTimeout    time.Duration
	ExchangeTimeout time.Duration
	BarrierTimeout  time.Duration
	StartDelay      time.Duration

	// MaxActiveAttempts limits the number of concurrently active dial_id buckets
	// served by ServeClient. This is a safety guard against unbounded dial_id
	// churn; it is not a network scale limit.
	MaxActiveAttempts int
}

type Session struct {
	cfg Config

	baseTopic string

	c     *mqttclient.Client
	msgCh chan *packet.Message
	errCh chan error

	mu sync.Mutex
}

func DeriveSID(proxyName, secretKey string) string {
	sum := sha256.Sum256([]byte(strings.TrimSpace(proxyName) + "\n" + strings.TrimSpace(secretKey)))
	// 16 bytes -> 32 hex chars is enough for P3.5 while keeping logs readable.
	return hex.EncodeToString(sum[:16])
}

func Open(ctx context.Context, cfg Config) (*Session, error) {
	if strings.TrimSpace(cfg.BrokerURL) == "" {
		return nil, errors.New("mqtt broker url is required")
	}
	if strings.TrimSpace(cfg.TopicPrefix) == "" {
		cfg.TopicPrefix = "miopunch/p3.5"
	}
	if strings.TrimSpace(cfg.SID) == "" {
		return nil, errors.New("mqtt sid is required")
	}
	if cfg.Role != RoleClient && cfg.Role != RoleVisitor {
		return nil, fmt.Errorf("invalid mqtt role: %q", cfg.Role)
	}
	if cfg.HelloInterval == 0 {
		cfg.HelloInterval = 500 * time.Millisecond
	}
	if cfg.HelloTimeout == 0 {
		cfg.HelloTimeout = 5 * time.Second
	}
	if cfg.ExchangeTimeout == 0 {
		cfg.ExchangeTimeout = 5 * time.Second
	}
	if cfg.BarrierTimeout == 0 {
		cfg.BarrierTimeout = 5 * time.Second
	}
	if cfg.StartDelay == 0 {
		cfg.StartDelay = 500 * time.Millisecond
	}
	if cfg.MaxActiveAttempts <= 0 {
		cfg.MaxActiveAttempts = 4096
	}

	s := &Session{
		cfg:   cfg,
		msgCh: make(chan *packet.Message, 256),
		errCh: make(chan error, 1),
	}
	s.baseTopic = strings.TrimRight(cfg.TopicPrefix, "/") + "/" + cfg.SID

	s.c = mqttclient.New()
	s.c.Callback = func(msg *packet.Message, err error) error {
		if err != nil {
			select {
			case s.errCh <- err:
			default:
			}
			return nil
		}
		if msg == nil {
			return nil
		}
		select {
		case s.msgCh <- msg:
		default:
		}
		return nil
	}

	clientID := fmt.Sprintf("miopunch-%s-%d", cfg.Role, time.Now().UTC().UnixNano())
	ccfg := mqttclient.NewConfigWithClientID(cfg.BrokerURL, clientID)
	ccfg.CleanSession = true
	ccfg.ValidateSubs = true

	f, err := s.c.Connect(ccfg)
	if err != nil {
		return nil, err
	}
	if err := waitFuture(ctx, f, cfg.HelloTimeout); err != nil {
		return nil, err
	}

	sub, err := s.c.Subscribe(s.baseTopic+"/#", packet.QOSAtLeastOnce)
	if err != nil {
		_ = s.c.Close()
		return nil, err
	}
	if err := waitFuture(ctx, sub, cfg.HelloTimeout); err != nil {
		_ = s.c.Close()
		return nil, err
	}
	return s, nil
}

func (s *Session) Close() error {
	if s == nil || s.c == nil {
		return nil
	}
	_ = s.c.Disconnect(500 * time.Millisecond)
	return s.c.Close()
}

func (s *Session) RunClient(ctx context.Context, m *wire.NatHoleClient) (*wire.NatHoleResp, error) {
	if m == nil {
		return nil, errors.New("nil NatHoleClient")
	}

	if err := s.helloBarrier(ctx); err != nil {
		return nil, err
	}

	dialID, visitorMsg, err := s.waitVisitorInfo(ctx)
	if err != nil {
		return nil, err
	}

	exchCtx, cancel := context.WithTimeout(ctx, s.cfg.ExchangeTimeout)
	defer cancel()

	clientMsg := cloneNatHoleClient(m)
	clientMsg.TransactionID = s.newClientAttemptTransactionID(dialID, visitorMsg)
	if err := s.publishJSON(exchCtx, s.attemptTopic(dialID, "info/client"), clientMsg); err != nil {
		return nil, err
	}

	var resp wire.NatHoleResp
	if err := s.waitJSON(exchCtx, s.attemptTopic(dialID, "resp/client"), &resp); err != nil {
		return nil, err
	}

	barCtx, cancel := context.WithTimeout(ctx, s.cfg.BarrierTimeout)
	defer cancel()
	if err := s.publishJSON(barCtx, s.attemptTopic(dialID, "ready/client"), map[string]any{"ts_unix_ms": time.Now().UnixMilli()}); err != nil {
		return nil, err
	}

	var start startMsg
	if err := s.waitJSON(barCtx, s.attemptTopic(dialID, "start"), &start); err != nil {
		return nil, err
	}
	if start.StartAtUnixMs <= 0 {
		return nil, errors.New("invalid start_at from mqtt")
	}
	waitUntil(time.UnixMilli(start.StartAtUnixMs))
	return &resp, nil
}

type AnalyzeFunc func(sid string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (visitorResp, clientResp *wire.NatHoleResp, err error)

func (s *Session) RunVisitor(ctx context.Context, m *wire.NatHoleVisitor, analyze AnalyzeFunc) (*wire.NatHoleResp, error) {
	if m == nil {
		return nil, errors.New("nil NatHoleVisitor")
	}
	if analyze == nil {
		return nil, errors.New("missing analyze func")
	}

	if err := s.helloBarrier(ctx); err != nil {
		return nil, err
	}

	dialID, err := normalizeDialID(m.TransactionID)
	if err != nil {
		return nil, err
	}

	exchCtx, cancel := context.WithTimeout(ctx, s.cfg.ExchangeTimeout)
	defer cancel()
	if err := s.publishJSON(exchCtx, s.attemptTopic(dialID, "info/visitor"), m); err != nil {
		return nil, err
	}

	var clientMsg wire.NatHoleClient
	if err := s.waitJSON(exchCtx, s.attemptTopic(dialID, "info/client"), &clientMsg); err != nil {
		return nil, err
	}

	visitorResp, clientResp, err := analyze(s.cfg.SID, m, &clientMsg)
	analyzeErr := err
	if analyzeErr != nil {
		visitorResp = &wire.NatHoleResp{
			TransactionID: m.TransactionID,
			Sid:           s.cfg.SID,
			Error:         analyzeErr.Error(),
		}
		clientResp = &wire.NatHoleResp{
			TransactionID: clientMsg.TransactionID,
			Sid:           s.cfg.SID,
			Error:         analyzeErr.Error(),
		}
	}

	if err := s.publishJSON(exchCtx, s.attemptTopic(dialID, "resp/client"), clientResp); err != nil {
		return nil, err
	}
	if err := s.publishJSON(exchCtx, s.attemptTopic(dialID, "resp/visitor"), visitorResp); err != nil {
		return nil, err
	}

	barCtx, cancel := context.WithTimeout(ctx, s.cfg.BarrierTimeout)
	defer cancel()
	if err := s.publishJSON(barCtx, s.attemptTopic(dialID, "ready/visitor"), map[string]any{"ts_unix_ms": time.Now().UnixMilli()}); err != nil {
		return nil, err
	}
	if err := s.waitAny(barCtx, s.attemptTopic(dialID, "ready/client")); err != nil {
		return nil, err
	}

	startAt := time.Now().Add(s.cfg.StartDelay)
	start := startMsg{StartAtUnixMs: startAt.UnixMilli()}
	if err := s.publishJSON(barCtx, s.attemptTopic(dialID, "start"), start); err != nil {
		return nil, err
	}
	waitUntil(startAt)
	if analyzeErr != nil {
		return visitorResp, analyzeErr
	}
	return visitorResp, nil
}

type ClientAttempt struct {
	DialID      string
	Visitor     *wire.NatHoleVisitor
	Client      *wire.NatHoleClient
	ClientResp  *wire.NatHoleResp
	StartedAtMs int64
	Err         error
}

// ServeClient runs a long-lived MQTT signaling accept loop for the "client" role.
//
// It binds each attempt by dial_id (derived from visitor's TransactionID) and runs
// per-attempt handlers concurrently. This is intended for daemon/acceptor mode,
// where a single client needs to serve multiple concurrent visitors.
//
// The handler MUST be non-blocking or fast; it runs in the per-attempt goroutine.
func (s *Session) ServeClient(ctx context.Context, template *wire.NatHoleClient, handle func(ClientAttempt)) error {
	if template == nil {
		return errors.New("nil NatHoleClient")
	}
	if handle == nil {
		return errors.New("missing client attempt handler")
	}

	serveCtx, cancel := context.WithCancel(ctx)
	defer cancel()

	// ServeClient is intended for long-lived daemon usage. The visitor side still
	// uses helloBarrier() to avoid a startup race with late subscriptions, so the
	// client must continuously publish hello/ack presence messages.
	go func() {
		ticker := time.NewTicker(s.cfg.HelloInterval)
		defer ticker.Stop()

		helloTopic := s.topic("hello/" + string(RoleClient))
		ackTopic := s.topic("hello_ack/" + string(RoleClient))

		publishPresence := func() {
			pubCtx, cancel := context.WithTimeout(serveCtx, s.cfg.HelloTimeout)
			defer cancel()
			_ = s.publishJSON(pubCtx, helloTopic, map[string]any{"ts_unix_ms": time.Now().UnixMilli()})
			_ = s.publishJSON(pubCtx, ackTopic, map[string]any{"ts_unix_ms": time.Now().UnixMilli()})
		}

		// Publish immediately to reduce time-to-first-visitor.
		publishPresence()

		for {
			select {
			case <-serveCtx.Done():
				return
			case <-ticker.C:
				publishPresence()
			}
		}
	}()

	type attemptState struct {
		dialID string
		inCh   chan *packet.Message
	}

	type attemptResult struct {
		dialID      string
		visitor     *wire.NatHoleVisitor
		client      *wire.NatHoleClient
		clientResp  *wire.NatHoleResp
		startedAtMs int64
		err         error
	}

	attempts := make(map[string]*attemptState)
	results := make(chan attemptResult, 32)

	var wg sync.WaitGroup
	defer func() {
		cancel()
		wg.Wait()
	}()

	startAttempt := func(dialID string, visitor *wire.NatHoleVisitor) {
		if _, ok := attempts[dialID]; ok {
			return
		}

		st := &attemptState{
			dialID: dialID,
			inCh:   make(chan *packet.Message, 128),
		}
		attempts[dialID] = st

		wg.Add(1)
		go func() {
			defer wg.Done()

			clientResp, clientMsg, startedAtMs, err := s.runClientAttempt(serveCtx, dialID, visitor, template, st.inCh)
			res := attemptResult{
				dialID:      dialID,
				visitor:     visitor,
				client:      clientMsg,
				clientResp:  clientResp,
				startedAtMs: startedAtMs,
				err:         err,
			}
			select {
			case results <- res:
			case <-serveCtx.Done():
			}
		}()
	}

	for {
		select {
		case <-serveCtx.Done():
			return nil
		case err := <-s.errCh:
			cancel()
			if err == nil {
				return errors.New("mqtt session error")
			}
			return err
		case msg := <-s.msgCh:
			if msg == nil {
				continue
			}

			if dialID, ok := parseDialIDFromVisitorInfoTopic(s.baseTopic, msg.Topic); ok {
				var visitor wire.NatHoleVisitor
				if err := json.Unmarshal(msg.Payload, &visitor); err != nil {
					continue
				}
				dialID, err := normalizeDialID(dialID)
				if err != nil {
					continue
				}
				if strings.TrimSpace(visitor.TransactionID) == "" {
					visitor.TransactionID = dialID
				}
				if s.cfg.MaxActiveAttempts > 0 && len(attempts) >= s.cfg.MaxActiveAttempts {
					visitorCopy := visitor
					res := attemptResult{
						dialID:  dialID,
						visitor: &visitorCopy,
						err:     ErrTooManyAttempts,
					}
					select {
					case results <- res:
					case <-serveCtx.Done():
					}
					continue
				}
				startAttempt(dialID, &visitor)
				continue
			}

			dialID, ok := parseDialIDFromAttemptTopic(s.baseTopic, msg.Topic)
			if !ok {
				continue
			}
			st, ok := attempts[dialID]
			if !ok {
				continue
			}
			select {
			case st.inCh <- msg:
			default:
			}
		case res := <-results:
			st, ok := attempts[res.dialID]
			if ok {
				delete(attempts, res.dialID)
				close(st.inCh)
			}
			handle(ClientAttempt{
				DialID:      res.dialID,
				Visitor:     res.visitor,
				Client:      res.client,
				ClientResp:  res.clientResp,
				StartedAtMs: res.startedAtMs,
				Err:         res.err,
			})
		}
	}
}

type startMsg struct {
	StartAtUnixMs int64 `json:"start_at_unix_ms"`
}

func (s *Session) helloBarrier(ctx context.Context) error {
	peer := RoleClient
	if s.cfg.Role == RoleClient {
		peer = RoleVisitor
	}

	helloTopic := s.topic("hello/" + string(s.cfg.Role))
	peerHelloTopic := s.topic("hello/" + string(peer))
	ackTopic := s.topic("hello_ack/" + string(s.cfg.Role))
	peerAckTopic := s.topic("hello_ack/" + string(peer))

	barCtx, cancel := context.WithTimeout(ctx, s.cfg.HelloTimeout)
	defer cancel()

	ticker := time.NewTicker(s.cfg.HelloInterval)
	defer ticker.Stop()

	// Two-way presence handshake:
	// - Always publish hello/<role> until the barrier completes.
	// - After observing peer's hello, publish hello_ack/<role> until complete.
	// This avoids a startup race where one side publishes hello before the other
	// has subscribed, then stops publishing too early.
	sawPeerHello := false
	sawPeerAck := false
	_ = s.publishJSON(barCtx, helloTopic, map[string]any{"ts_unix_ms": time.Now().UnixMilli()})
	for {
		select {
		case <-barCtx.Done():
			return fmt.Errorf("mqtt hello barrier timeout (%s)", s.cfg.HelloTimeout)
		case err := <-s.errCh:
			return err
		case msg := <-s.msgCh:
			if msg.Topic == peerHelloTopic {
				sawPeerHello = true
				_ = s.publishJSON(barCtx, ackTopic, map[string]any{"ts_unix_ms": time.Now().UnixMilli()})
			}
			if msg.Topic == peerAckTopic {
				sawPeerAck = true
			}
			if sawPeerHello && sawPeerAck {
				return nil
			}
		case <-ticker.C:
			_ = s.publishJSON(barCtx, helloTopic, map[string]any{"ts_unix_ms": time.Now().UnixMilli()})
			if sawPeerHello {
				_ = s.publishJSON(barCtx, ackTopic, map[string]any{"ts_unix_ms": time.Now().UnixMilli()})
			}
		}
	}
}

func (s *Session) topic(suffix string) string {
	return s.baseTopic + "/" + strings.TrimLeft(suffix, "/")
}

func (s *Session) attemptTopic(dialID string, suffix string) string {
	dialID, err := normalizeDialID(dialID)
	if err != nil {
		// We keep this helper panic-free for production flows; callers validate
		// dialID up-front. Falling back to a clearly invalid topic makes the
		// publish/wait paths fail deterministically.
		return s.baseTopic + "/attempt/INVALID_DIAL_ID/" + strings.TrimLeft(suffix, "/")
	}
	return s.baseTopic + "/attempt/" + dialID + "/" + strings.TrimLeft(suffix, "/")
}

func (s *Session) publishJSON(ctx context.Context, topic string, v any) error {
	data, err := json.Marshal(v)
	if err != nil {
		return err
	}
	f, err := s.c.Publish(topic, data, packet.QOSAtLeastOnce, false)
	if err != nil {
		return err
	}
	return waitFuture(ctx, f, s.cfg.ExchangeTimeout)
}

func (s *Session) waitJSON(ctx context.Context, topic string, out any) error {
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case err := <-s.errCh:
			return err
		case msg := <-s.msgCh:
			if msg.Topic != topic {
				continue
			}
			if err := json.Unmarshal(msg.Payload, out); err != nil {
				return err
			}
			return nil
		}
	}
}

func (s *Session) waitAny(ctx context.Context, topic string) error {
	var ignored any
	return s.waitJSON(ctx, topic, &ignored)
}

type attemptVisitorInfo struct {
	DialID   string
	Visitor  *wire.NatHoleVisitor
	RecvAt   time.Time
	RawTopic string
}

func (s *Session) waitVisitorInfo(ctx context.Context) (string, *wire.NatHoleVisitor, error) {
	barCtx, cancel := context.WithTimeout(ctx, s.cfg.ExchangeTimeout)
	defer cancel()

	for {
		select {
		case <-barCtx.Done():
			return "", nil, barCtx.Err()
		case err := <-s.errCh:
			return "", nil, err
		case msg := <-s.msgCh:
			dialID, ok := parseDialIDFromVisitorInfoTopic(s.baseTopic, msg.Topic)
			if !ok {
				continue
			}
			var v wire.NatHoleVisitor
			if err := json.Unmarshal(msg.Payload, &v); err != nil {
				return "", nil, err
			}
			dialID, err := normalizeDialID(dialID)
			if err != nil {
				return "", nil, err
			}
			if strings.TrimSpace(v.TransactionID) == "" {
				v.TransactionID = dialID
			}
			return dialID, &v, nil
		}
	}
}

func (s *Session) runClientAttempt(ctx context.Context, dialID string, visitor *wire.NatHoleVisitor, template *wire.NatHoleClient, in <-chan *packet.Message) (*wire.NatHoleResp, *wire.NatHoleClient, int64, error) {
	dialID, err := normalizeDialID(dialID)
	if err != nil {
		return nil, nil, 0, err
	}

	exchCtx, cancel := context.WithTimeout(ctx, s.cfg.ExchangeTimeout)
	defer cancel()

	clientMsg := cloneNatHoleClient(template)
	clientMsg.TransactionID = s.newClientAttemptTransactionID(dialID, visitor)
	if err := s.publishJSON(exchCtx, s.attemptTopic(dialID, "info/client"), clientMsg); err != nil {
		return nil, clientMsg, 0, err
	}

	var resp wire.NatHoleResp
	if err := waitJSONFromInbound(exchCtx, in, s.attemptTopic(dialID, "resp/client"), &resp); err != nil {
		return nil, clientMsg, 0, err
	}

	barCtx, cancel := context.WithTimeout(ctx, s.cfg.BarrierTimeout)
	defer cancel()
	if err := s.publishJSON(barCtx, s.attemptTopic(dialID, "ready/client"), map[string]any{"ts_unix_ms": time.Now().UnixMilli()}); err != nil {
		return nil, clientMsg, 0, err
	}

	var start startMsg
	if err := waitJSONFromInbound(barCtx, in, s.attemptTopic(dialID, "start"), &start); err != nil {
		return nil, clientMsg, 0, err
	}
	if start.StartAtUnixMs <= 0 {
		return nil, clientMsg, 0, errors.New("invalid start_at from mqtt")
	}
	waitUntil(time.UnixMilli(start.StartAtUnixMs))
	return &resp, clientMsg, start.StartAtUnixMs, nil
}

func normalizeDialID(dialID string) (string, error) {
	dialID = strings.TrimSpace(dialID)
	if dialID == "" {
		return "", errors.New("missing dial_id")
	}
	if strings.Contains(dialID, "/") {
		return "", fmt.Errorf("invalid dial_id: %q", dialID)
	}
	return dialID, nil
}

func parseDialIDFromVisitorInfoTopic(baseTopic string, topic string) (string, bool) {
	prefix := strings.TrimRight(baseTopic, "/") + "/attempt/"
	if !strings.HasPrefix(topic, prefix) {
		return "", false
	}
	rest := strings.TrimPrefix(topic, prefix)
	dialID, suffix, ok := strings.Cut(rest, "/")
	if !ok || strings.TrimSpace(dialID) == "" {
		return "", false
	}
	if strings.TrimLeft(suffix, "/") != "info/visitor" {
		return "", false
	}
	return dialID, true
}

func parseDialIDFromAttemptTopic(baseTopic string, topic string) (string, bool) {
	prefix := strings.TrimRight(baseTopic, "/") + "/attempt/"
	if !strings.HasPrefix(topic, prefix) {
		return "", false
	}
	rest := strings.TrimPrefix(topic, prefix)
	dialID, _, ok := strings.Cut(rest, "/")
	if !ok || strings.TrimSpace(dialID) == "" {
		return "", false
	}
	return dialID, true
}

func waitJSONFromInbound(ctx context.Context, in <-chan *packet.Message, topic string, out any) error {
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case msg, ok := <-in:
			if !ok {
				return io.ErrClosedPipe
			}
			if msg == nil || msg.Topic != topic {
				continue
			}
			if err := json.Unmarshal(msg.Payload, out); err != nil {
				return err
			}
			return nil
		}
	}
}

func cloneNatHoleClient(in *wire.NatHoleClient) *wire.NatHoleClient {
	if in == nil {
		return nil
	}
	cp := *in
	cp.Capabilities = append([]string(nil), in.Capabilities...)
	cp.DirectAddrs = append([]string(nil), in.DirectAddrs...)
	cp.MappedAddrs = append([]string(nil), in.MappedAddrs...)
	cp.AssistedAddrs = append([]string(nil), in.AssistedAddrs...)
	cp.TCPDirectAddrs = append([]string(nil), in.TCPDirectAddrs...)
	cp.TCPAssistedAddrs = append([]string(nil), in.TCPAssistedAddrs...)
	cp.TCPMappedAddrs = append([]string(nil), in.TCPMappedAddrs...)
	return &cp
}

func (s *Session) newClientAttemptTransactionID(dialID string, visitor *wire.NatHoleVisitor) string {
	// The client_tx is only used for correlation inside resp/client. Keep it
	// unique per attempt to avoid confusing concurrent exchanges.
	s.mu.Lock()
	defer s.mu.Unlock()
	return fmt.Sprintf("client-%s-%d", dialID, time.Now().UTC().UnixNano())
}

func waitFuture(ctx context.Context, f mqttclient.GenericFuture, fallback time.Duration) error {
	timeout := fallback
	if deadline, ok := ctx.Deadline(); ok {
		remaining := time.Until(deadline)
		if remaining > 0 && remaining < timeout {
			timeout = remaining
		}
	}
	err := f.Wait(timeout)
	if err == future.ErrTimeout {
		return context.DeadlineExceeded
	}
	return err
}

func waitUntil(t time.Time) {
	delay := time.Until(t)
	if delay <= 0 {
		return
	}
	timer := time.NewTimer(delay)
	<-timer.C
	timer.Stop()
}
