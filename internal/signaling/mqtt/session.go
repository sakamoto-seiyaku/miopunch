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
	"strings"
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
}

type Session struct {
	cfg Config

	baseTopic string

	c     *mqttclient.Client
	msgCh chan *packet.Message
	errCh chan error
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

	exchCtx, cancel := context.WithTimeout(ctx, s.cfg.ExchangeTimeout)
	defer cancel()
	if err := s.publishJSON(exchCtx, s.topic("info/client"), m); err != nil {
		return nil, err
	}
	var resp wire.NatHoleResp
	if err := s.waitJSON(exchCtx, s.topic("resp/client"), &resp); err != nil {
		return nil, err
	}

	barCtx, cancel := context.WithTimeout(ctx, s.cfg.BarrierTimeout)
	defer cancel()
	if err := s.publishJSON(barCtx, s.topic("ready/client"), map[string]any{"ts_unix_ms": time.Now().UnixMilli()}); err != nil {
		return nil, err
	}

	var start startMsg
	if err := s.waitJSON(barCtx, s.topic("start"), &start); err != nil {
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

	exchCtx, cancel := context.WithTimeout(ctx, s.cfg.ExchangeTimeout)
	defer cancel()
	if err := s.publishJSON(exchCtx, s.topic("info/visitor"), m); err != nil {
		return nil, err
	}

	var clientMsg wire.NatHoleClient
	if err := s.waitJSON(exchCtx, s.topic("info/client"), &clientMsg); err != nil {
		return nil, err
	}

	visitorResp, clientResp, err := analyze(s.cfg.SID, m, &clientMsg)
	if err != nil {
		return nil, err
	}

	if err := s.publishJSON(exchCtx, s.topic("resp/client"), clientResp); err != nil {
		return nil, err
	}
	if err := s.publishJSON(exchCtx, s.topic("resp/visitor"), visitorResp); err != nil {
		return nil, err
	}

	barCtx, cancel := context.WithTimeout(ctx, s.cfg.BarrierTimeout)
	defer cancel()
	if err := s.publishJSON(barCtx, s.topic("ready/visitor"), map[string]any{"ts_unix_ms": time.Now().UnixMilli()}); err != nil {
		return nil, err
	}
	if err := s.waitAny(barCtx, s.topic("ready/client")); err != nil {
		return nil, err
	}

	startAt := time.Now().Add(s.cfg.StartDelay)
	start := startMsg{StartAtUnixMs: startAt.UnixMilli()}
	if err := s.publishJSON(barCtx, s.topic("start"), start); err != nil {
		return nil, err
	}
	waitUntil(startAt)
	return visitorResp, nil
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
