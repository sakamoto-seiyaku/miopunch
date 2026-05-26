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

package presence

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"
	"time"

	mqttclient "github.com/256dpi/gomqtt/client"
	"github.com/256dpi/gomqtt/client/future"
	"github.com/256dpi/gomqtt/packet"
)

const defaultTimeout = 5 * time.Second

// Session owns one broker-backed current v1 presence producer/consumer.
type Session struct {
	cfg            Config
	engine         *Engine
	client         *mqttclient.Client
	presenceTopic  string
	subscribeTopic string

	mu     sync.Mutex
	closed bool
	signal chan struct{}
	errs   chan error
}

// OpenSession connects to the configured broker, subscribes to the retained
// presence snapshot, and publishes retained online state.
func OpenSession(ctx context.Context, cfg Config) (*Session, error) {
	normalizedCfg, err := normalizeConfig(cfg)
	if err != nil {
		return nil, err
	}

	engine, err := NewEngine(normalizedCfg)
	if err != nil {
		return nil, err
	}

	presenceTopic, err := normalizedCfg.TopicScope.PresenceTopic(normalizedCfg.SelfPeerID)
	if err != nil {
		return nil, err
	}

	offlinePayload, err := EncodeSelfPayload(normalizedCfg, OnlineStateOffline, uint64(time.Now().UTC().UnixMilli()))
	if err != nil {
		return nil, err
	}

	s := &Session{
		cfg:            normalizedCfg,
		engine:         engine,
		client:         mqttclient.New(),
		presenceTopic:  presenceTopic,
		subscribeTopic: PresenceWildcardTopic(normalizedCfg.TopicScope),
		signal:         make(chan struct{}, 1),
		errs:           make(chan error, 1),
	}

	s.client.Callback = func(msg *packet.Message, err error) error {
		if err != nil {
			s.sendErr(err)
			return nil
		}
		if msg == nil {
			return nil
		}

		s.mu.Lock()
		changed := s.engine.ApplyMessage(msg.Topic, msg.Payload)
		s.mu.Unlock()
		if changed {
			s.notify()
		}
		return nil
	}

	clientID := fmt.Sprintf("miopunch-pocv1-presence-%s-%d", strings.ToLower(normalizedCfg.SelfPeerID), time.Now().UTC().UnixNano())
	ccfg := mqttclient.NewConfigWithClientID(brokerURL(normalizedCfg.RuntimeBroker.Endpoint), clientID)
	ccfg.CleanSession = true
	ccfg.ValidateSubs = true
	ccfg.WillMessage = &packet.Message{
		Topic:   presenceTopic,
		Payload: offlinePayload,
		QOS:     packet.QOSAtLeastOnce,
		Retain:  true,
	}

	connectFuture, err := s.client.Connect(ccfg)
	if err != nil {
		return nil, err
	}
	if err := waitFuture(ctx, connectFuture, defaultTimeout); err != nil {
		_ = s.client.Close()
		return nil, err
	}

	subscribeFuture, err := s.client.Subscribe(s.subscribeTopic, packet.QOSAtLeastOnce)
	if err != nil {
		_ = s.client.Close()
		return nil, err
	}
	if err := waitFuture(ctx, subscribeFuture, defaultTimeout); err != nil {
		_ = s.client.Close()
		return nil, err
	}

	if err := s.publishState(ctx, OnlineStateOnline); err != nil {
		_ = s.client.Close()
		return nil, err
	}

	return s, nil
}

// View returns the current discover view snapshot.
func (s *Session) View() DiscoverView {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.engine.View(time.Now())
}

// Diagnostics returns the current accumulated presence diagnostics.
func (s *Session) Diagnostics() []Diagnostic {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.engine.Diagnostics()
}

// WaitForPeerState waits until the discover view contains peerID in wantState.
func (s *Session) WaitForPeerState(
	ctx context.Context,
	peerID string,
	wantState OnlineState,
) (DiscoverView, error) {
	for {
		view := s.View()
		if hasPeerState(view, peerID, wantState) {
			return view, nil
		}

		select {
		case <-ctx.Done():
			return DiscoverView{}, ctx.Err()
		case err := <-s.errs:
			if err == nil {
				return DiscoverView{}, errors.New("presence session error")
			}
			return DiscoverView{}, fmt.Errorf("wait for peer state %q=%q: %w", peerID, wantState, err)
		case <-s.signal:
		}
	}
}

// Close publishes retained offline state and disconnects cleanly.
func (s *Session) Close(ctx context.Context) error {
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		return nil
	}
	s.closed = true
	s.mu.Unlock()

	if err := s.publishState(ctx, OnlineStateOffline); err != nil {
		_ = s.client.Close()
		return fmt.Errorf("publish offline presence during close: %w", err)
	}

	if err := s.client.Disconnect(500 * time.Millisecond); err != nil && !errors.Is(err, mqttclient.ErrClientNotConnected) {
		return fmt.Errorf("disconnect presence session: %w", err)
	}
	return nil
}

// Abort closes the client without a graceful retained offline publish.
func (s *Session) Abort() error {
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		return nil
	}
	s.closed = true
	s.mu.Unlock()

	if err := s.client.Close(); err != nil && !errors.Is(err, mqttclient.ErrClientNotConnected) {
		return fmt.Errorf("abort presence session: %w", err)
	}
	return nil
}

func (s *Session) publishState(ctx context.Context, state OnlineState) error {
	payload, err := EncodeSelfPayload(s.cfg, state, uint64(time.Now().UTC().UnixMilli()))
	if err != nil {
		return fmt.Errorf("encode %q presence payload: %w", state, err)
	}

	publishFuture, err := s.client.Publish(s.presenceTopic, payload, packet.QOSAtLeastOnce, true)
	if err != nil {
		return fmt.Errorf("publish %q presence state: %w", state, err)
	}
	if err := waitFuture(ctx, publishFuture, defaultTimeout); err != nil {
		return fmt.Errorf("wait for %q presence publish: %w", state, err)
	}
	return nil
}

func (s *Session) notify() {
	select {
	case s.signal <- struct{}{}:
	default:
	}
}

func (s *Session) sendErr(err error) {
	select {
	case s.errs <- err:
	default:
	}
}

func hasPeerState(view DiscoverView, peerID string, wantState OnlineState) bool {
	for _, peer := range view.Peers {
		if peer.PeerID == peerID && peer.OnlineState == wantState {
			return true
		}
	}
	return false
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
