package dataplane

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"time"

	"github.com/apernet/quic-go"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/netutil"
	"github.com/miopunch/miopunch/internal/tlsutil"
)

// DialStream establishes the selected data plane over the already-working UDP path
// and returns a stream suitable for long-lived bidirectional I/O.
//
// The returned ReadWriteCloser owns the underlying UDP socket and MUST be closed.
func DialStream(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, em *event.Emitter) (io.ReadWriteCloser, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	switch cfg.Proto {
	case ProtocolKCP:
		return dialKCPStream(ctx, listenConn, raddr)
	case ProtocolQUIC:
		return dialQUICStream(ctx, cfg, listenConn, raddr, em)
	default:
		return nil, fmt.Errorf("unknown data proto: %q", cfg.Proto)
	}
}

// ServeStream accepts / serves the selected data plane over the already-working UDP path
// and returns a stream suitable for long-lived bidirectional I/O.
//
// For KCP, raddr MUST be provided (the already-known remote UDP address).
// The returned ReadWriteCloser owns the underlying UDP socket and MUST be closed.
func ServeStream(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, em *event.Emitter) (io.ReadWriteCloser, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	switch cfg.Proto {
	case ProtocolKCP:
		return serveKCPStream(ctx, listenConn, raddr)
	case ProtocolQUIC:
		return serveQUICStream(ctx, cfg, listenConn, em)
	default:
		return nil, fmt.Errorf("unknown data proto: %q", cfg.Proto)
	}
}

type quicStreamConn struct {
	udp       *net.UDPConn
	conn      *quic.Conn
	stream    *quic.Stream
	closeConn bool
}

func (q *quicStreamConn) Read(p []byte) (int, error)  { return q.stream.Read(p) }
func (q *quicStreamConn) Write(p []byte) (int, error) { return q.stream.Write(p) }
func (q *quicStreamConn) Close() error {
	if q == nil {
		return nil
	}
	if q.stream != nil {
		_ = q.stream.Close()
	}

	if q.conn != nil && !q.closeConn {
		if q.udp != nil {
			udp := q.udp
			conn := q.conn
			q.udp = nil
			go func() {
				select {
				case <-conn.Context().Done():
				case <-time.After(5 * time.Second):
				}
				_ = udp.Close()
			}()
		}
		return nil
	}

	if q.conn != nil {
		q.conn.CloseWithError(0, "")
	}
	if q.udp != nil {
		_ = q.udp.Close()
	}
	return nil
}

func dialQUICStream(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, em *event.Emitter) (io.ReadWriteCloser, error) {
	if listenConn == nil {
		return nil, errors.New("quic requires listen conn")
	}
	if raddr == nil {
		return nil, errors.New("quic requires remote addr")
	}

	tlsConfig, err := tlsutil.NewClientTLSConfig("", "", "", raddr.String())
	if err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	tlsConfig.NextProtos = []string{dataALPN}

	conn, err := quic.Dial(ctx, listenConn, raddr, tlsConfig, &quic.Config{
		HandshakeIdleTimeout: 20 * time.Second,
		MaxIdleTimeout:       30 * time.Second,
		KeepAlivePeriod:      10 * time.Second,
	})
	if err != nil {
		_ = listenConn.Close()
		return nil, err
	}

	if err := applyQUICCC(cfg, conn); err != nil {
		conn.CloseWithError(0, "")
		_ = listenConn.Close()
		return nil, err
	}

	stream, err := conn.OpenStreamSync(ctx)
	if err != nil {
		conn.CloseWithError(0, "")
		_ = listenConn.Close()
		return nil, err
	}

	if em != nil {
		em.Emit(event.Event{
			Stage: event.StageTransport,
			Kind:  event.KindStart,
			Name:  "transport.stream_open",
			Msg:   "quic stream open",
			KVs: map[string]any{
				"data_proto": string(cfg.Proto),
				"quic_cc":    string(cfg.QuicCC),
			},
		})
	}

	return &quicStreamConn{
		udp:       listenConn,
		conn:      conn,
		stream:    stream,
		closeConn: true,
	}, nil
}

func serveQUICStream(ctx context.Context, cfg Config, listenConn *net.UDPConn, em *event.Emitter) (io.ReadWriteCloser, error) {
	if listenConn == nil {
		return nil, errors.New("quic requires listen conn")
	}

	tlsConfig, err := tlsutil.NewServerTLSConfig("", "", "")
	if err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	tlsConfig.NextProtos = []string{dataALPN}

	ln, err := quic.Listen(listenConn, tlsConfig, &quic.Config{
		HandshakeIdleTimeout: 20 * time.Second,
		MaxIdleTimeout:       30 * time.Second,
		KeepAlivePeriod:      10 * time.Second,
	})
	if err != nil {
		_ = listenConn.Close()
		return nil, err
	}

	conn, err := ln.Accept(ctx)
	if err != nil {
		_ = ln.Close()
		_ = listenConn.Close()
		return nil, err
	}

	if err := applyQUICCC(cfg, conn); err != nil {
		conn.CloseWithError(0, "")
		_ = ln.Close()
		_ = listenConn.Close()
		return nil, err
	}

	stream, err := conn.AcceptStream(ctx)
	if err != nil {
		conn.CloseWithError(0, "")
		_ = ln.Close()
		_ = listenConn.Close()
		return nil, err
	}

	_ = ln.Close()

	if em != nil {
		em.Emit(event.Event{
			Stage: event.StageTransport,
			Kind:  event.KindStart,
			Name:  "transport.stream_accept",
			Msg:   "quic stream accepted",
			KVs: map[string]any{
				"data_proto": string(cfg.Proto),
				"quic_cc":    string(cfg.QuicCC),
			},
		})
	}

	return &quicStreamConn{
		udp:    listenConn,
		conn:   conn,
		stream: stream,
	}, nil
}

type kcpStreamConn struct {
	udp  *net.UDPConn
	conn net.Conn
}

func (k *kcpStreamConn) Read(p []byte) (int, error)  { return k.conn.Read(p) }
func (k *kcpStreamConn) Write(p []byte) (int, error) { return k.conn.Write(p) }
func (k *kcpStreamConn) Close() error {
	if k == nil {
		return nil
	}
	if k.conn != nil {
		_ = k.conn.Close()
	}
	if k.udp != nil {
		_ = k.udp.Close()
	}
	return nil
}

func dialKCPStream(ctx context.Context, listenConn *net.UDPConn, raddr *net.UDPAddr) (io.ReadWriteCloser, error) {
	if listenConn == nil {
		return nil, errors.New("kcp requires listen conn")
	}
	if err := ctx.Err(); err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	if raddr == nil {
		_ = listenConn.Close()
		return nil, errors.New("kcp requires remote addr")
	}

	// Mirror dataplane/kcp.go: re-dial a connected UDP socket to enforce remote
	// filtering and make kcp-go behave like a stream transport.
	laddr, err := net.ResolveUDPAddr("udp", listenConn.LocalAddr().String())
	if err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	_ = listenConn.Close()

	udpConn, err := net.DialUDP("udp", laddr, raddr)
	if err != nil {
		return nil, err
	}
	if deadline, ok := ctx.Deadline(); ok {
		if err := udpConn.SetDeadline(deadline); err != nil {
			_ = udpConn.Close()
			return nil, err
		}
	}

	c, err := netutil.NewKCPConnFromUDP(udpConn, true, raddr.String())
	if err != nil {
		_ = udpConn.Close()
		return nil, err
	}
	if deadline, ok := ctx.Deadline(); ok {
		if err := c.SetDeadline(deadline); err != nil {
			_ = c.Close()
			_ = udpConn.Close()
			return nil, err
		}
	}

	return &kcpStreamConn{udp: udpConn, conn: c}, nil
}

func serveKCPStream(ctx context.Context, listenConn *net.UDPConn, raddr *net.UDPAddr) (io.ReadWriteCloser, error) {
	return dialKCPStream(ctx, listenConn, raddr)
}
