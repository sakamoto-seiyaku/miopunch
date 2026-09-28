package udpowner

import (
	"context"
	"errors"
	"net"
	"strings"
	"sync"
	"time"

	"github.com/apernet/quic-go"
	"golang.org/x/net/ipv4"

	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/punchwire"
	"github.com/miopunch/miopunch/internal/wire"
)

// TraversalEndpoint is a per-transaction demuxed view over traversal packets.
//
// It is safe for concurrent use.
//
// Implementations MUST NOT read from the underlying UDP socket directly. All UDP
// receives are owned by the socket owner / demux loop.
type TraversalEndpoint interface {
	Recv(ctx context.Context, b []byte) (int, *net.UDPAddr, error)
	// SendTo sends a traversal packet to addr.
	//
	// ttl is best-effort and applies to IPv4 only. The implementation may fall
	// back to the socket's default TTL if it can't be set.
	SendTo(ctx context.Context, b []byte, addr *net.UDPAddr, ttl int) error
	Close() error
}

type packet struct {
	data []byte
	addr *net.UDPAddr
}

var errEndpointClosed = errors.New("traversal endpoint closed")

// TraversalDemux owns traversal receives and routes tagged punching packets to
// per-transaction endpoints.
//
// This demux assumes all traversal packets are encoded as wire.NatHoleSid.
type TraversalDemux struct {
	key []byte

	recv    func(ctx context.Context, b []byte) (int, *net.UDPAddr, error)
	sendRaw func(b []byte, addr *net.UDPAddr) error

	// sendMu serializes all sends so a temporary TTL override doesn't leak to
	// concurrent packets on the same socket.
	sendMu  sync.Mutex
	ttlConn *net.UDPConn

	ctx    context.Context
	cancel context.CancelFunc

	mu   sync.Mutex
	byTx map[string]chan packet

	closed chan struct{}
	wg     sync.WaitGroup

	onClose func()
}

type DemuxConfig struct {
	// Key is used to decrypt traversal packets and extract TransactionID.
	Key []byte
	// QueueLen is the per-transaction queue capacity.
	// When full, packets are dropped to avoid blocking the owner receive loop.
	QueueLen int
}

func (c *DemuxConfig) normalize() {
	if c.QueueLen <= 0 {
		c.QueueLen = 8
	}
}

// NewQUICTraversalDemux creates a traversal demux that receives packets via
// quic.Transport.ReadNonQUICPacket and sends packets via quic.Transport.WriteTo.
func NewQUICTraversalDemux(tr *quic.Transport, cfg DemuxConfig) (*TraversalDemux, error) {
	if tr == nil {
		return nil, errors.New("nil quic transport")
	}
	cfg.normalize()
	ctx, cancel := context.WithCancel(context.Background())
	d := &TraversalDemux{
		key:    cfg.Key,
		closed: make(chan struct{}),
		byTx:   make(map[string]chan packet),
		ctx:    ctx,
		cancel: cancel,
		recv: func(ctx context.Context, b []byte) (int, *net.UDPAddr, error) {
			n, addr, err := tr.ReadNonQUICPacket(ctx, b)
			if err != nil {
				return 0, nil, err
			}
			udpAddr, ok := addr.(*net.UDPAddr)
			if !ok {
				return 0, nil, errors.New("non-udp addr")
			}
			return n, udpAddr, nil
		},
		sendRaw: func(b []byte, addr *net.UDPAddr) error {
			_, err := tr.WriteTo(b, addr)
			return err
		},
	}
	if c, ok := tr.Conn.(*net.UDPConn); ok {
		d.ttlConn = c
	}

	d.wg.Add(1)
	go func() {
		defer d.wg.Done()
		d.run()
	}()
	return d, nil
}

// NewUDPTraversalDemux creates a traversal demux that receives packets via
// UDPConn.ReadFromUDP and sends via UDPConn.WriteToUDP.
func NewUDPTraversalDemux(conn *net.UDPConn, cfg DemuxConfig) (*TraversalDemux, error) {
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
			for {
				if err := ctx.Err(); err != nil {
					return 0, nil, err
				}
				_ = conn.SetReadDeadline(time.Now().Add(250 * time.Millisecond))
				n, addr, err := conn.ReadFromUDP(b)
				_ = conn.SetReadDeadline(time.Time{})
				if err != nil {
					if udpReadTimeoutError(err) {
						continue
					}
					if recoverableUDPReadError(err) {
						logutil.Tracef("udp traversal recovered from read error: err=%v", err)
						continue
					}
					return 0, nil, err
				}
				return n, addr, nil
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

func (d *TraversalDemux) Close() error {
	if d == nil {
		return nil
	}
	select {
	case <-d.closed:
		return nil
	default:
		close(d.closed)
	}
	if d.cancel != nil {
		d.cancel()
	}
	if d.onClose != nil {
		d.onClose()
	}

	d.mu.Lock()
	for tx, ch := range d.byTx {
		delete(d.byTx, tx)
		close(ch)
	}
	d.mu.Unlock()

	d.wg.Wait()
	return nil
}

func (d *TraversalDemux) Open(transactionID string, queueLen int) TraversalEndpoint {
	transactionID = strings.TrimSpace(transactionID)
	if transactionID == "" {
		return &closedEndpoint{}
	}
	if queueLen <= 0 {
		queueLen = 8
	}

	d.mu.Lock()
	defer d.mu.Unlock()
	if d.byTx == nil {
		return &closedEndpoint{}
	}
	if _, ok := d.byTx[transactionID]; ok {
		// A tx should be unique; keep the existing endpoint.
		return &closedEndpoint{}
	}
	ch := make(chan packet, queueLen)
	d.byTx[transactionID] = ch
	return &demuxEndpoint{
		d:  d,
		tx: transactionID,
		ch: ch,
	}
}

func (d *TraversalDemux) unregister(tx string) {
	d.mu.Lock()
	ch := d.byTx[tx]
	delete(d.byTx, tx)
	d.mu.Unlock()
	if ch != nil {
		close(ch)
	}
}

func (d *TraversalDemux) run() {
	buf := make([]byte, 2048)

	for {
		select {
		case <-d.closed:
			return
		default:
		}

		n, raddr, err := d.recv(d.ctx, buf)
		if err != nil {
			if d.ctx.Err() != nil {
				return
			}
			select {
			case <-d.closed:
				return
			default:
			}
			// Treat transient errors as stop signals. The owner is torn down by
			// closing the underlying socket/transport elsewhere.
			return
		}
		if n <= 0 || raddr == nil {
			continue
		}

		// Fast-path: traversal packets must be tagged.
		if !punchwire.HasPunchTag(buf[:n]) {
			continue
		}
		logutil.Tracef("udp traversal packet received: remote=%s bytes=%d", raddr.String(), n)

		var m wire.NatHoleSid
		if err := punchwire.DecodeMessageInto(buf[:n], d.key, &m); err != nil {
			logutil.Tracef("udp traversal decode failed: remote=%s bytes=%d err=%v", raddr.String(), n, err)
			continue
		}
		tx := strings.TrimSpace(m.TransactionID)
		if tx == "" {
			logutil.Tracef("udp traversal missing transaction: remote=%s sid=%s response=%t", raddr.String(), m.Sid, m.Response)
			continue
		}

		data := make([]byte, n)
		copy(data, buf[:n])

		d.mu.Lock()
		ch := d.byTx[tx]
		d.mu.Unlock()
		if ch == nil {
			logutil.Tracef("udp traversal unknown transaction: tx=%s sid=%s response=%t remote=%s", tx, m.Sid, m.Response, raddr.String())
			// For unknown transaction IDs, best-effort respond to requests so the
			// peer's attempt can still progress (the peer will correlate by its tx).
			if !m.Response {
				m.Response = true
				if out, err := punchwire.EncodeMessage(&m, d.key); err == nil {
					if err := d.send(context.Background(), out, raddr, 0); err != nil {
						logutil.Tracef("udp traversal unknown transaction auto-response failed: tx=%s sid=%s remote=%s err=%v", tx, m.Sid, raddr.String(), err)
					} else {
						logutil.Tracef("udp traversal unknown transaction auto-response sent: tx=%s sid=%s remote=%s", tx, m.Sid, raddr.String())
					}
				} else {
					logutil.Tracef("udp traversal unknown transaction auto-response encode failed: tx=%s sid=%s remote=%s err=%v", tx, m.Sid, raddr.String(), err)
				}
			}
			continue
		}
		select {
		case ch <- packet{data: data, addr: raddr}:
			logutil.Tracef("udp traversal routed packet: tx=%s sid=%s response=%t remote=%s", tx, m.Sid, m.Response, raddr.String())
		default:
			// Drop when full to keep owner recv loop unblocked.
			logutil.Tracef("udp traversal endpoint queue full: tx=%s sid=%s response=%t remote=%s", tx, m.Sid, m.Response, raddr.String())
		}
	}
}

func (d *TraversalDemux) send(ctx context.Context, b []byte, addr *net.UDPAddr, ttl int) error {
	if d == nil {
		return errEndpointClosed
	}
	if addr == nil {
		return errors.New("nil udp addr")
	}

	d.sendMu.Lock()
	defer d.sendMu.Unlock()

	var (
		uConn  *ipv4.Conn
		orig   int
		origOK bool
		setErr error
	)
	if ttl > 0 && d.ttlConn != nil && addr.IP != nil && addr.IP.To4() != nil {
		uConn = ipv4.NewConn(d.ttlConn)
		if v, err := uConn.TTL(); err == nil && v > 0 && v <= 255 {
			orig = v
			origOK = true
		}
		setErr = uConn.SetTTL(ttl)
	}

	err := d.sendRaw(b, addr)

	if ttl > 0 && uConn != nil && setErr == nil {
		restoreTo := 64
		if origOK {
			restoreTo = orig
		}
		_ = uConn.SetTTL(restoreTo)
	}

	_ = ctx
	return err
}

type demuxEndpoint struct {
	d  *TraversalDemux
	tx string
	ch chan packet

	once sync.Once
}

func (e *demuxEndpoint) Recv(ctx context.Context, b []byte) (int, *net.UDPAddr, error) {
	if e == nil || e.ch == nil {
		return 0, nil, errEndpointClosed
	}
	select {
	case <-ctx.Done():
		return 0, nil, ctx.Err()
	case p, ok := <-e.ch:
		if !ok {
			return 0, nil, errEndpointClosed
		}
		n := copy(b, p.data)
		return n, p.addr, nil
	}
}

func (e *demuxEndpoint) SendTo(ctx context.Context, b []byte, addr *net.UDPAddr, ttl int) error {
	if e == nil || e.d == nil {
		return errEndpointClosed
	}
	return e.d.send(ctx, b, addr, ttl)
}

func (e *demuxEndpoint) Close() error {
	if e == nil || e.d == nil {
		return nil
	}
	e.once.Do(func() {
		e.d.unregister(e.tx)
	})
	return nil
}

type closedEndpoint struct{}

func (closedEndpoint) Recv(ctx context.Context, b []byte) (int, *net.UDPAddr, error) {
	_ = ctx
	_ = b
	return 0, nil, errEndpointClosed
}

func (closedEndpoint) SendTo(ctx context.Context, b []byte, addr *net.UDPAddr, ttl int) error {
	_ = ctx
	_ = b
	_ = addr
	_ = ttl
	return errEndpointClosed
}

func (closedEndpoint) Close() error { return nil }
