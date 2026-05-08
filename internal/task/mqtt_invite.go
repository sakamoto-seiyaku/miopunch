package task

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
	"github.com/256dpi/gomqtt/transport"
)

type mqttMailbox struct {
	endpoint string
	url      string

	c     *mqttclient.Client
	msgCh chan *packet.Message
	errCh chan error
}

func openMQTTMailboxes(ctx context.Context, endpoints []string, clientIDPrefix string) ([]*mqttMailbox, error) {
	mbs := make([]*mqttMailbox, 0, len(endpoints))
	for _, ep := range endpoints {
		mb, err := openMQTTMailbox(ctx, ep, clientIDPrefix)
		if err != nil {
			closeMQTTMailboxes(mbs)
			return nil, fmt.Errorf("%s: %w", strings.TrimSpace(ep), err)
		}
		mbs = append(mbs, mb)
	}
	if len(mbs) == 0 {
		return nil, errors.New("no broker mailboxes opened")
	}
	return mbs, nil
}

func checkMQTTBrokersReachable(ctx context.Context, endpoints []string, clientIDPrefix string) error {
	mbs, err := openMQTTMailboxes(ctx, endpoints, clientIDPrefix)
	if err != nil {
		return err
	}
	closeMQTTMailboxes(mbs)
	return nil
}

func closeMQTTMailboxes(mbs []*mqttMailbox) {
	for _, mb := range mbs {
		_ = mb.Close()
	}
}

func openMQTTMailbox(ctx context.Context, endpoint string, clientIDPrefix string) (*mqttMailbox, error) {
	endpoint = strings.TrimSpace(endpoint)
	if endpoint == "" {
		return nil, errors.New("empty broker endpoint")
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	url := endpoint
	if !strings.Contains(url, "://") {
		url = "tcp://" + endpoint
	}

	mb := &mqttMailbox{
		endpoint: endpoint,
		url:      url,
		c:        mqttclient.New(),
		msgCh:    make(chan *packet.Message, 1),
		errCh:    make(chan error, 1),
	}

	mb.c.Callback = func(msg *packet.Message, err error) error {
		if err != nil {
			select {
			case mb.errCh <- err:
			default:
			}
			return nil
		}
		if msg == nil {
			return nil
		}
		select {
		case mb.msgCh <- msg:
		default:
		}
		return nil
	}

	clientID := fmt.Sprintf("%s-%d", strings.TrimSpace(clientIDPrefix), time.Now().UTC().UnixNano())
	cfg := mqttclient.NewConfigWithClientID(url, clientID)
	cfg.CleanSession = true
	cfg.ValidateSubs = true

	dialTimeout := 5 * time.Second
	if deadline, ok := ctx.Deadline(); ok {
		remaining := time.Until(deadline)
		if remaining <= 0 {
			return nil, context.DeadlineExceeded
		}
		if remaining < dialTimeout {
			dialTimeout = remaining
		}
	}
	cfg.Dialer = transport.NewDialer(transport.DialConfig{Timeout: dialTimeout})

	f, err := mb.c.Connect(cfg)
	if err != nil {
		return nil, err
	}
	if err := waitMQTTFutureCtx(ctx, f, 5*time.Second); err != nil {
		_ = mb.Close()
		return nil, err
	}

	return mb, nil
}

func (mb *mqttMailbox) Close() error {
	if mb == nil || mb.c == nil {
		return nil
	}
	_ = mb.c.Disconnect(200 * time.Millisecond)
	return mb.c.Close()
}

func (mb *mqttMailbox) Subscribe(ctx context.Context, topic string) error {
	if mb == nil || mb.c == nil {
		return errors.New("nil mqtt mailbox")
	}
	topic = strings.TrimSpace(topic)
	if topic == "" {
		return errors.New("empty subscribe topic")
	}
	f, err := mb.c.Subscribe(topic, packet.QOSAtLeastOnce)
	if err != nil {
		return err
	}
	return waitMQTTFutureCtx(ctx, f, 5*time.Second)
}

func (mb *mqttMailbox) Publish(ctx context.Context, topic string, payload []byte) error {
	if mb == nil || mb.c == nil {
		return errors.New("nil mqtt mailbox")
	}
	topic = strings.TrimSpace(topic)
	if topic == "" {
		return errors.New("empty publish topic")
	}
	f, err := mb.c.Publish(topic, payload, packet.QOSAtLeastOnce, false)
	if err != nil {
		return err
	}
	return waitMQTTFutureCtx(ctx, f, 5*time.Second)
}

type mailboxEvent struct {
	Endpoint string
	Topic    string
	Payload  []byte
	Err      error
}

func fanInMailboxEvents(ctx context.Context, mbs []*mqttMailbox) (<-chan mailboxEvent, func()) {
	ctx, cancel := context.WithCancel(ctx)
	out := make(chan mailboxEvent, 1)

	var wg sync.WaitGroup
	stop := func() {
		cancel()
		for _, mb := range mbs {
			_ = mb.Close()
		}
		wg.Wait()
		close(out)
	}

	for _, mb := range mbs {
		mb := mb
		if mb == nil {
			continue
		}

		wg.Add(1)
		go func() {
			defer wg.Done()

			for {
				select {
				case <-ctx.Done():
					return
				case err := <-mb.errCh:
					select {
					case out <- mailboxEvent{Endpoint: mb.endpoint, Err: err}:
					case <-ctx.Done():
					}
					return
				case msg := <-mb.msgCh:
					if msg == nil {
						continue
					}

					// Copy: mqtt client owns backing bytes.
					payload := make([]byte, len(msg.Payload))
					copy(payload, msg.Payload)
					select {
					case out <- mailboxEvent{Endpoint: mb.endpoint, Topic: msg.Topic, Payload: payload}:
					case <-ctx.Done():
					}
				}
			}
		}()
	}

	return out, stop
}

func waitMQTTFutureCtx(ctx context.Context, f mqttclient.GenericFuture, fallback time.Duration) error {
	timeout := fallback
	if deadline, ok := ctx.Deadline(); ok {
		remaining := time.Until(deadline)
		if remaining > 0 && remaining < timeout {
			timeout = remaining
		}
	}
	err := f.Wait(timeout)
	if errors.Is(err, future.ErrTimeout) {
		return context.DeadlineExceeded
	}
	return err
}
