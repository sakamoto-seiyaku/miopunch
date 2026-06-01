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
	"errors"
	"fmt"
	"strings"
	"time"

	mqttclient "github.com/256dpi/gomqtt/client"
	"github.com/256dpi/gomqtt/packet"

	"github.com/miopunch/miopunch/internal/pocv1/peere2e"
	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
)

const defaultPeerMessageTimeout = 5 * time.Second

// PeerMessageConfig configures one binary TLV peer-message MQTT session.
type PeerMessageConfig struct {
	BrokerURL       string
	SubscribeTopics []string
}

// OpenedPeerMessage is one successfully opened current v1 peer message.
type OpenedPeerMessage struct {
	Topic string
	Outer pocwire.OuterHeader
	Inner pocwire.InnerMessage
}

// PeerMessageSession publishes and receives current v1 peer-targeted MQTT
// payload bytes without going through the legacy JSON wire structs.
type PeerMessageSession struct {
	cfg PeerMessageConfig

	c     *mqttclient.Client
	msgCh chan *packet.Message
	errCh chan error
}

// OpenPeerMessageSession connects to one broker and subscribes to the exact
// topics used by the current v1 peer-targeted runtime path.
func OpenPeerMessageSession(ctx context.Context, cfg PeerMessageConfig) (*PeerMessageSession, error) {
	if strings.TrimSpace(cfg.BrokerURL) == "" {
		return nil, errors.New("mqtt broker url is required")
	}

	s := &PeerMessageSession{
		cfg:   cfg,
		msgCh: make(chan *packet.Message, 256),
		errCh: make(chan error, 1),
	}

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

	clientID := fmt.Sprintf("miopunch-peer-message-%d", time.Now().UTC().UnixNano())
	ccfg := mqttclient.NewConfigWithClientID(cfg.BrokerURL, clientID)
	ccfg.CleanSession = true
	ccfg.ValidateSubs = true

	f, err := s.c.Connect(ccfg)
	if err != nil {
		return nil, err
	}
	if err := waitFuture(ctx, f, defaultPeerMessageTimeout); err != nil {
		return nil, err
	}

	seenTopics := make(map[string]struct{}, len(cfg.SubscribeTopics))
	for _, topic := range cfg.SubscribeTopics {
		topic = strings.TrimSpace(topic)
		if topic == "" {
			_ = s.c.Close()
			return nil, errors.New("mqtt subscribe topic is required")
		}
		if _, ok := seenTopics[topic]; ok {
			continue
		}
		seenTopics[topic] = struct{}{}

		sub, err := s.c.Subscribe(topic, packet.QOSAtLeastOnce)
		if err != nil {
			_ = s.c.Close()
			return nil, err
		}
		if err := waitFuture(ctx, sub, defaultPeerMessageTimeout); err != nil {
			_ = s.c.Close()
			return nil, err
		}
	}

	return s, nil
}

// Close disconnects the MQTT client.
func (s *PeerMessageSession) Close() error {
	if s == nil || s.c == nil {
		return nil
	}
	_ = s.c.Disconnect(500 * time.Millisecond)
	return s.c.Close()
}

// PublishInner seals one signed current v1 inner message and publishes the
// resulting outer TLV bytes as the MQTT payload.
func (s *PeerMessageSession) PublishInner(
	ctx context.Context,
	topic string,
	inner pocwire.InnerMessage,
	recipientX25519PublicKey []byte,
	opts peere2e.SealOptions,
) (pocwire.OuterHeader, error) {
	topic = strings.TrimSpace(topic)
	if topic == "" {
		return pocwire.OuterHeader{}, errors.New("mqtt topic is required")
	}

	outer, err := pocwire.OuterForInner(inner)
	if err != nil {
		return pocwire.OuterHeader{}, err
	}

	sealedOuter, err := peere2e.Seal(outer, inner, recipientX25519PublicKey, opts)
	if err != nil {
		return pocwire.OuterHeader{}, err
	}

	payload, err := sealedOuter.MarshalBinary()
	if err != nil {
		return pocwire.OuterHeader{}, err
	}
	if err := s.publishPayload(ctx, topic, payload); err != nil {
		return pocwire.OuterHeader{}, err
	}
	return sealedOuter, nil
}

// PublishPayload publishes an already-sealed peer-message payload to the given topic.
func (s *PeerMessageSession) PublishPayload(ctx context.Context, topic string, payload []byte) error {
	topic = strings.TrimSpace(topic)
	if topic == "" {
		return errors.New("mqtt topic is required")
	}
	if len(payload) == 0 {
		return errors.New("mqtt payload is required")
	}
	return s.publishPayload(ctx, topic, payload)
}

// WaitOpened waits for the next valid current v1 peer-targeted payload, drops
// malformed or unauthenticated inputs locally, and returns the first opened
// message.
func (s *PeerMessageSession) WaitOpened(
	ctx context.Context,
	recipientX25519PrivateKey []byte,
	opts peere2e.OpenOptions,
) (OpenedPeerMessage, error) {
	for {
		msg, err := s.waitMessage(ctx)
		if err != nil {
			return OpenedPeerMessage{}, err
		}

		outer, err := pocwire.UnmarshalOuterHeader(msg.Payload)
		if err != nil {
			if shouldDropPeerMessage(err) {
				continue
			}
			return OpenedPeerMessage{}, err
		}

		inner, err := peere2e.Open(outer, recipientX25519PrivateKey, opts)
		if err != nil {
			if shouldDropPeerMessage(err) {
				continue
			}
			return OpenedPeerMessage{}, err
		}

		return OpenedPeerMessage{
			Topic: msg.Topic,
			Outer: outer,
			Inner: inner,
		}, nil
	}
}

func (s *PeerMessageSession) publishPayload(ctx context.Context, topic string, payload []byte) error {
	f, err := s.c.Publish(topic, payload, packet.QOSAtLeastOnce, false)
	if err != nil {
		return err
	}
	return waitFuture(ctx, f, defaultPeerMessageTimeout)
}

func (s *PeerMessageSession) waitMessage(ctx context.Context) (*packet.Message, error) {
	for {
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case err := <-s.errCh:
			if err == nil {
				return nil, errors.New("mqtt peer message session error")
			}
			return nil, err
		case msg := <-s.msgCh:
			if msg == nil {
				continue
			}
			return msg, nil
		}
	}
}

func shouldDropPeerMessage(err error) bool {
	return errors.Is(err, pocwire.ErrMalformedTLV) ||
		errors.Is(err, pocwire.ErrUnknownTag) ||
		errors.Is(err, pocwire.ErrDuplicateTag) ||
		errors.Is(err, pocwire.ErrNonCanonicalUvarint) ||
		errors.Is(err, pocwire.ErrOutOfOrderField) ||
		errors.Is(err, pocwire.ErrTruncatedTLV) ||
		errors.Is(err, pocwire.ErrInvalidASCII) ||
		errors.Is(err, pocwire.ErrInvalidFieldValue) ||
		errors.Is(err, pocwire.ErrUnsupportedVersion) ||
		errors.Is(err, pocwire.ErrInvalidScheme) ||
		errors.Is(err, pocwire.ErrUnsupportedKind) ||
		errors.Is(err, pocwire.ErrInvalidSignature) ||
		errors.Is(err, pocwire.ErrOuterInnerMismatch) ||
		errors.Is(err, pocwire.ErrExpired) ||
		errors.Is(err, pocwire.ErrReplay) ||
		errors.Is(err, peere2e.ErrInvalidFrame) ||
		errors.Is(err, peere2e.ErrDecryptFailed)
}
