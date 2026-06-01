package udpowner

import (
	"context"
	"encoding/binary"
	"errors"
	"net"
	"sync"
	"sync/atomic"
	"time"

	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/punchwire"
)

// KCPOwner is a UDP socket owner / demux implementation for KCP mode.
//
// It enforces a single ReadFromUDP loop and splits packets by PunchTagV1:
// - tagged traversal packets go to a TraversalDemux (per-transaction buckets)
// - everything else is exposed as a net.PacketConn for kcp-go.
type KCPOwner struct {
	conn *net.UDPConn

	closed      chan struct{}
	closeOnce   sync.Once
	chCloseOnce sync.Once
	wg          sync.WaitGroup

	traversalIn chan packet
	kcpIn       chan packet

	traversal *TraversalDemux
	kcpPC     *kcpPacketConn

	kcpEnqueued       atomic.Uint64
	kcpDequeued       atomic.Uint64
	kcpDropped        atomic.Uint64
	kcpFECData        atomic.Uint64
	kcpFECParity      atomic.Uint64
	kcpConv1          atomic.Uint64
	kcpConvOther      atomic.Uint64
	kcpLenMin         atomic.Uint64
	kcpLenMax         atomic.Uint64
	kcpLenLT32        atomic.Uint64
	kcpWrites         atomic.Uint64
	kcpWriteBytes     atomic.Uint64
	traversalEnqueued atomic.Uint64
	traversalDropped  atomic.Uint64
}

type KCPOwnerConfig struct {
	Traversal DemuxConfig
	// KCPQueueLen is the inbound queue size for KCP packets.
	// When full, packets are dropped to keep the single-owner read loop unblocked.
	KCPQueueLen int
	// TraversalQueueLen is the inbound queue size for traversal packets (tagged).
	// When full, packets are dropped to keep the single-owner read loop unblocked.
	TraversalQueueLen int
}

func (c *KCPOwnerConfig) normalize() {
	c.Traversal.normalize()
	if c.KCPQueueLen <= 0 {
		c.KCPQueueLen = 128
	}
	if c.TraversalQueueLen <= 0 {
		c.TraversalQueueLen = 32
	}
}

func NewKCPOwner(conn *net.UDPConn, cfg KCPOwnerConfig) (*KCPOwner, error) {
	if conn == nil {
		return nil, errors.New("nil udp conn")
	}
	cfg.normalize()

	o := &KCPOwner{
		conn:        conn,
		closed:      make(chan struct{}),
		traversalIn: make(chan packet, cfg.TraversalQueueLen),
		kcpIn:       make(chan packet, cfg.KCPQueueLen),
	}
	o.kcpPC = newKCPPacketConn(o, conn.LocalAddr())

	// Traversal demux reads from traversalIn channel (fed by the owner read loop).
	d, err := newChanTraversalDemux(o.traversalIn, conn, cfg.Traversal)
	if err != nil {
		return nil, err
	}
	o.traversal = d

	o.wg.Add(1)
	go func() {
		defer o.wg.Done()
		o.run()
	}()

	return o, nil
}

func (o *KCPOwner) PacketConn() net.PacketConn { return o.kcpPC }

func (o *KCPOwner) TraversalDemux() *TraversalDemux { return o.traversal }

type KCPOwnerStats struct {
	KCPEnqueued       uint64
	KCPDequeued       uint64
	KCPDropped        uint64
	KCPFECData        uint64
	KCPFECParity      uint64
	KCPConv1          uint64
	KCPConvOther      uint64
	KCPLenMin         uint64
	KCPLenMax         uint64
	KCPLenLT32        uint64
	KCPWrites         uint64
	KCPWriteBytes     uint64
	TraversalEnqueued uint64
	TraversalDropped  uint64
}

func (o *KCPOwner) Stats() KCPOwnerStats {
	if o == nil {
		return KCPOwnerStats{}
	}
	return KCPOwnerStats{
		KCPEnqueued:       o.kcpEnqueued.Load(),
		KCPDequeued:       o.kcpDequeued.Load(),
		KCPDropped:        o.kcpDropped.Load(),
		KCPFECData:        o.kcpFECData.Load(),
		KCPFECParity:      o.kcpFECParity.Load(),
		KCPConv1:          o.kcpConv1.Load(),
		KCPConvOther:      o.kcpConvOther.Load(),
		KCPLenMin:         o.kcpLenMin.Load(),
		KCPLenMax:         o.kcpLenMax.Load(),
		KCPLenLT32:        o.kcpLenLT32.Load(),
		KCPWrites:         o.kcpWrites.Load(),
		KCPWriteBytes:     o.kcpWriteBytes.Load(),
		TraversalEnqueued: o.traversalEnqueued.Load(),
		TraversalDropped:  o.traversalDropped.Load(),
	}
}

func (o *KCPOwner) Close() error {
	if o == nil {
		return nil
	}
	o.closeOnce.Do(func() {
		close(o.closed)
		// Closing the UDPConn unblocks ReadFromUDP in the owner loop.
		if o.conn != nil {
			_ = o.conn.Close()
		}
	})
	o.wg.Wait()

	if o.traversal != nil {
		_ = o.traversal.Close()
	}

	// Close channels after the owner loop stops sending.
	//
	// Note: owner Close() may be reached via multiple closers (e.g. a listener
	// closing its PacketConn, plus an explicit owner Close). Make it idempotent.
	o.chCloseOnce.Do(func() {
		if o.traversalIn != nil {
			close(o.traversalIn)
		}
		if o.kcpIn != nil {
			close(o.kcpIn)
		}
	})
	return nil
}

func (o *KCPOwner) run() {
	buf := make([]byte, 2048)
	for {
		n, raddr, err := o.conn.ReadFromUDP(buf)
		if err != nil {
			select {
			case <-o.closed:
				return
			default:
			}
			return
		}
		if n <= 0 || raddr == nil {
			continue
		}

		data := make([]byte, n)
		copy(data, buf[:n])
		p := packet{data: data, addr: raddr}

		if punchwire.HasPunchTag(data) {
			select {
			case <-o.closed:
				return
			case o.traversalIn <- p:
				o.traversalEnqueued.Add(1)
				logutil.Tracef("udp owner routed tagged traversal packet: remote=%s bytes=%d", raddr.String(), n)
			default:
				// Drop tagged traversal packets on backpressure.
				o.traversalDropped.Add(1)
				logutil.Tracef("udp owner traversal queue full drop: remote=%s bytes=%d", raddr.String(), n)
			}
			continue
		}

		select {
		case <-o.closed:
			return
		case o.kcpIn <- p:
			o.kcpEnqueued.Add(1)
			if n < 32 {
				o.kcpLenLT32.Add(1)
			}
			{
				// best-effort min/max tracking for debugging and observability
				ln := uint64(n)
				for {
					cur := o.kcpLenMin.Load()
					if cur != 0 && cur <= ln {
						break
					}
					if o.kcpLenMin.CompareAndSwap(cur, ln) {
						break
					}
				}
				for {
					cur := o.kcpLenMax.Load()
					if cur >= ln {
						break
					}
					if o.kcpLenMax.CompareAndSwap(cur, ln) {
						break
					}
				}
			}
			if len(data) >= 6 {
				switch binary.LittleEndian.Uint16(data[4:]) {
				case 0xf1:
					o.kcpFECData.Add(1)
					if len(data) >= 12 {
						if binary.LittleEndian.Uint32(data[8:12]) == 1 {
							o.kcpConv1.Add(1)
						} else {
							o.kcpConvOther.Add(1)
						}
					}
				case 0xf2:
					o.kcpFECParity.Add(1)
				}
			}
		default:
			// Drop KCP packets on backpressure.
			o.kcpDropped.Add(1)
		}
	}
}

func newChanTraversalDemux(in <-chan packet, conn *net.UDPConn, cfg DemuxConfig) (*TraversalDemux, error) {
	if conn == nil {
		return nil, errors.New("nil udp conn")
	}
	cfg.normalize()
	ctx, cancel := context.WithCancel(context.Background())
	d := &TraversalDemux{
		key:     cfg.Key,
		closed:  make(chan struct{}),
		byTx:    make(map[string]chan packet),
		ttlConn: conn,
		ctx:     ctx,
		cancel:  cancel,
		recv: func(ctx context.Context, b []byte) (int, *net.UDPAddr, error) {
			select {
			case <-ctx.Done():
				return 0, nil, ctx.Err()
			case p, ok := <-in:
				if !ok {
					return 0, nil, net.ErrClosed
				}
				n := copy(b, p.data)
				return n, p.addr, nil
			}
		},
		sendRaw: func(b []byte, addr *net.UDPAddr) error {
			_, err := conn.WriteToUDP(b, addr)
			return err
		},
	}

	d.wg.Add(1)
	go func() {
		defer d.wg.Done()
		d.run()
	}()
	return d, nil
}

// kcpPacketConn exposes the non-traversal packets of a KCPOwner as a net.PacketConn.
// It is intended to be passed to kcp-go (ServeConn / NewConn3).
type kcpPacketConn struct {
	o     *KCPOwner
	local net.Addr

	mu         sync.Mutex
	deadline   time.Time
	rdDeadline time.Time
	wrDeadline time.Time
}

func newKCPPacketConn(o *KCPOwner, local net.Addr) *kcpPacketConn {
	return &kcpPacketConn{o: o, local: local}
}

func (c *kcpPacketConn) ReadFrom(p []byte) (n int, addr net.Addr, err error) {
	if c == nil || c.o == nil {
		return 0, nil, net.ErrClosed
	}

	deadline := c.readDeadline()
	if !deadline.IsZero() {
		d := time.Until(deadline)
		if d <= 0 {
			return 0, nil, &timeoutError{}
		}
		timer := time.NewTimer(d)
		defer timer.Stop()
		select {
		case <-c.o.closed:
			return 0, nil, net.ErrClosed
		case pkt, ok := <-c.o.kcpIn:
			if !ok {
				return 0, nil, net.ErrClosed
			}
			c.o.kcpDequeued.Add(1)
			n = copy(p, pkt.data)
			return n, pkt.addr, nil
		case <-timer.C:
			return 0, nil, &timeoutError{}
		}
	}

	select {
	case <-c.o.closed:
		return 0, nil, net.ErrClosed
	case pkt, ok := <-c.o.kcpIn:
		if !ok {
			return 0, nil, net.ErrClosed
		}
		c.o.kcpDequeued.Add(1)
		n = copy(p, pkt.data)
		return n, pkt.addr, nil
	}
}

func (c *kcpPacketConn) WriteTo(p []byte, addr net.Addr) (n int, err error) {
	if c == nil || c.o == nil || c.o.conn == nil {
		return 0, net.ErrClosed
	}
	if addr == nil {
		return 0, errors.New("nil addr")
	}

	udpAddr, ok := addr.(*net.UDPAddr)
	if !ok {
		return 0, errors.New("non-udp addr")
	}

	deadline := c.writeDeadline()
	if !deadline.IsZero() && time.Until(deadline) <= 0 {
		return 0, &timeoutError{}
	}

	c.o.kcpWrites.Add(1)
	c.o.kcpWriteBytes.Add(uint64(len(p)))
	return c.o.conn.WriteToUDP(p, udpAddr)
}

func (c *kcpPacketConn) Close() error {
	if c == nil || c.o == nil {
		return nil
	}
	return c.o.Close()
}

func (c *kcpPacketConn) LocalAddr() net.Addr { return c.local }

func (c *kcpPacketConn) SetDeadline(t time.Time) error {
	c.mu.Lock()
	c.deadline = t
	c.mu.Unlock()
	return nil
}

func (c *kcpPacketConn) SetReadDeadline(t time.Time) error {
	c.mu.Lock()
	c.rdDeadline = t
	c.mu.Unlock()
	return nil
}

func (c *kcpPacketConn) SetWriteDeadline(t time.Time) error {
	c.mu.Lock()
	c.wrDeadline = t
	c.mu.Unlock()
	return nil
}

func (c *kcpPacketConn) readDeadline() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return earliestNonZeroTime(c.deadline, c.rdDeadline)
}

func (c *kcpPacketConn) writeDeadline() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return earliestNonZeroTime(c.deadline, c.wrDeadline)
}

func earliestNonZeroTime(a, b time.Time) time.Time {
	switch {
	case a.IsZero():
		return b
	case b.IsZero():
		return a
	case a.Before(b):
		return a
	default:
		return b
	}
}

// timeoutError matches net's i/o timeout behavior (implements net.Error).
type timeoutError struct{}

func (e *timeoutError) Error() string   { return "i/o timeout" }
func (e *timeoutError) Timeout() bool   { return true }
func (e *timeoutError) Temporary() bool { return true }
func (e *timeoutError) Is(err error) bool {
	return err == context.DeadlineExceeded
}
