package stunclient

import (
	"context"
	"errors"
	"net"
	"slices"
	"sync"
	"time"

	"github.com/pion/stun/v2"
)

const udpReadLoopPollInterval = 100 * time.Millisecond

type UDPClient struct {
	conn     *net.UDPConn
	stopCh   chan struct{}
	doneCh   chan struct{}
	stopOnce sync.Once

	mu      sync.Mutex
	pending map[string]chan []byte
}

func NewUDPClient(conn *net.UDPConn) *UDPClient {
	c := &UDPClient{
		conn:    conn,
		stopCh:  make(chan struct{}),
		doneCh:  make(chan struct{}),
		pending: make(map[string]chan []byte),
	}
	go c.readLoop()
	return c
}

func (c *UDPClient) Close() {
	c.stopOnce.Do(func() {
		close(c.stopCh)
		_ = c.conn.SetReadDeadline(time.Now())
		<-c.doneCh
	})
}

func (c *UDPClient) RoundTrip(ctx context.Context, raddr *net.UDPAddr, req *stun.Message) (stun.Message, error) {
	key := stunTxKey(req.TransactionID)
	respCh := make(chan []byte, 1)
	if err := c.register(key, respCh); err != nil {
		return stun.Message{}, err
	}
	defer c.unregister(key)

	if _, err := c.conn.WriteToUDP(req.Raw, raddr); err != nil {
		return stun.Message{}, err
	}

	select {
	case raw := <-respCh:
		var msg stun.Message
		msg.Raw = raw
		if err := msg.Decode(); err != nil {
			return stun.Message{}, err
		}
		if msg.Type.Method != stun.MethodBinding {
			return stun.Message{}, errors.New("unexpected stun method")
		}
		if !slices.Equal(msg.TransactionID[:], req.TransactionID[:]) {
			return stun.Message{}, errors.New("stun transaction id mismatch")
		}
		return msg, nil
	case <-ctx.Done():
		return stun.Message{}, ctx.Err()
	case <-c.stopCh:
		return stun.Message{}, context.Canceled
	}
}

func (c *UDPClient) register(key string, respCh chan []byte) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	select {
	case <-c.stopCh:
		return context.Canceled
	default:
	}
	c.pending[key] = respCh
	return nil
}

func (c *UDPClient) unregister(key string) {
	c.mu.Lock()
	delete(c.pending, key)
	c.mu.Unlock()
}

func (c *UDPClient) readLoop() {
	defer close(c.doneCh)
	defer c.clearPending()
	defer func() { _ = c.conn.SetReadDeadline(time.Time{}) }()

	buf := make([]byte, 2048)
	for {
		select {
		case <-c.stopCh:
			return
		default:
		}

		_ = c.conn.SetReadDeadline(time.Now().Add(udpReadLoopPollInterval))
		n, _, err := c.conn.ReadFromUDP(buf)
		if err != nil {
			var netErr net.Error
			if errors.As(err, &netErr) && netErr.Timeout() {
				continue
			}
			select {
			case <-c.stopCh:
				return
			default:
				continue
			}
		}

		var msg stun.Message
		msg.Raw = append(msg.Raw[:0], buf[:n]...)
		if err := msg.Decode(); err != nil {
			continue
		}
		if msg.Type.Method != stun.MethodBinding {
			continue
		}

		key := stunTxKey(msg.TransactionID)
		c.mu.Lock()
		respCh := c.pending[key]
		c.mu.Unlock()
		if respCh == nil {
			continue
		}

		raw := append([]byte(nil), buf[:n]...)
		select {
		case respCh <- raw:
		default:
		}
	}
}

func (c *UDPClient) clearPending() {
	c.mu.Lock()
	defer c.mu.Unlock()
	for key := range c.pending {
		delete(c.pending, key)
	}
}

func stunTxKey(txID [stun.TransactionIDSize]byte) string {
	return string(txID[:])
}
