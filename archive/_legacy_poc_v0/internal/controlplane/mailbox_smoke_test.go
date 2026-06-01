package controlplane

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"net"
	"testing"
	"time"
	"unicode/utf8"

	"github.com/256dpi/gomqtt/broker"
	mqttclient "github.com/256dpi/gomqtt/client"
	"github.com/256dpi/gomqtt/client/future"
	"github.com/256dpi/gomqtt/packet"
	"github.com/256dpi/gomqtt/transport"
)

func TestMailboxSmoke_DerivedInboxTopicPublishSubscribe(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	dialer := startInMemoryMQTTBroker(t)

	netSecret := []byte("0123456789abcdef0123456789abcdef")
	peerID := base32RawNoPad.EncodeToString([]byte("0123456789abcdef"))

	inboxTopic, err := DeriveInboxTopic(netSecret, peerID)
	if err != nil {
		t.Fatalf("DeriveInboxTopic(%v, %q) error = %v", netSecret, peerID, err)
	}

	sub := newTestMQTTClient(t, dialer, "sub")
	pub := newTestMQTTClient(t, dialer, "pub")

	if err := sub.Subscribe(ctx, inboxTopic); err != nil {
		t.Fatalf("Subscribe(%q) error = %v", inboxTopic, err)
	}

	// Avoid putting readable plaintext into MQTT payloads in this smoke test.
	payload := []byte{
		0xff, 0xfe, 0xfd, 0xfc, 0x00, 0x01, 0x02, 0x03,
		0x10, 0x11, 0x12, 0x13, 0x7f, 0x80, 0x81, 0x82,
	}
	if utf8.Valid(payload) {
		t.Fatalf("payload is unexpectedly valid UTF-8: %x", payload)
	}

	if err := pub.Publish(ctx, inboxTopic, payload); err != nil {
		t.Fatalf("Publish(%q) error = %v", inboxTopic, err)
	}

	msg, err := sub.WaitTopic(ctx, inboxTopic)
	if err != nil {
		t.Fatalf("WaitTopic(%q) error = %v", inboxTopic, err)
	}
	if msg.Topic != inboxTopic {
		t.Fatalf("received topic = %q, want %q", msg.Topic, inboxTopic)
	}
	if !bytes.Equal(msg.Payload, payload) {
		t.Fatalf("received payload = %x, want %x", msg.Payload, payload)
	}
}

func startInMemoryMQTTBroker(t *testing.T) mqttclient.Dialer {
	t.Helper()

	backend := broker.NewMemoryBackend()
	engine := broker.NewEngine(backend)

	t.Cleanup(func() {
		backend.Close(500 * time.Millisecond)
	})

	return &inMemoryMQTTDialer{engine: engine}
}

type testMQTTClient struct {
	c *mqttclient.Client

	msgCh chan *packet.Message
	errCh chan error
}

func newTestMQTTClient(t *testing.T, dialer mqttclient.Dialer, label string) *testMQTTClient {
	t.Helper()

	tc := &testMQTTClient{
		c:     mqttclient.New(),
		msgCh: make(chan *packet.Message, 1),
		errCh: make(chan error, 1),
	}

	tc.c.Callback = func(msg *packet.Message, err error) error {
		if err != nil {
			select {
			case tc.errCh <- err:
			default:
			}
			return nil
		}
		if msg == nil {
			return nil
		}
		select {
		case tc.msgCh <- msg:
		default:
		}
		return nil
	}

	clientID := fmt.Sprintf("miopunch-mailbox-smoke-%s-%d", label, time.Now().UTC().UnixNano())
	cfg := mqttclient.NewConfigWithClientID("tcp://127.0.0.1:1883", clientID)
	cfg.Dialer = dialer
	cfg.CleanSession = true
	cfg.ValidateSubs = true

	f, err := tc.c.Connect(cfg)
	if err != nil {
		t.Fatalf("mqtt Connect() error = %v", err)
	}
	if err := waitMQTTClientFuture(f, 2*time.Second); err != nil {
		t.Fatalf("mqtt Connect() wait error = %v", err)
	}

	t.Cleanup(func() {
		_ = tc.c.Disconnect(200 * time.Millisecond)
		_ = tc.c.Close()
	})

	return tc
}

func (tc *testMQTTClient) Subscribe(ctx context.Context, topic string) error {
	f, err := tc.c.Subscribe(topic, packet.QOSAtLeastOnce)
	if err != nil {
		return err
	}
	return waitMQTTClientFutureCtx(ctx, f, 2*time.Second)
}

func (tc *testMQTTClient) Publish(ctx context.Context, topic string, payload []byte) error {
	f, err := tc.c.Publish(topic, payload, packet.QOSAtLeastOnce, false)
	if err != nil {
		return err
	}
	return waitMQTTClientFutureCtx(ctx, f, 2*time.Second)
}

func (tc *testMQTTClient) WaitTopic(ctx context.Context, topic string) (*packet.Message, error) {
	for {
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case err := <-tc.errCh:
			return nil, err
		case msg := <-tc.msgCh:
			if msg.Topic == topic {
				return msg, nil
			}
		}
	}
}

func waitMQTTClientFuture(f mqttclient.GenericFuture, timeout time.Duration) error {
	if err := f.Wait(timeout); err != nil {
		if errors.Is(err, future.ErrTimeout) {
			return context.DeadlineExceeded
		}
		return err
	}
	return nil
}

func waitMQTTClientFutureCtx(ctx context.Context, f mqttclient.GenericFuture, fallback time.Duration) error {
	timeout := fallback
	if deadline, ok := ctx.Deadline(); ok {
		remaining := time.Until(deadline)
		if remaining > 0 && remaining < timeout {
			timeout = remaining
		}
	}
	return waitMQTTClientFuture(f, timeout)
}

type inMemoryMQTTDialer struct {
	engine *broker.Engine
}

func (d *inMemoryMQTTDialer) Dial(_ string) (transport.Conn, error) {
	clientConn, serverConn := net.Pipe()

	go d.engine.Handle(transport.NewNetConn(serverConn))

	return transport.NewNetConn(clientConn), nil
}
