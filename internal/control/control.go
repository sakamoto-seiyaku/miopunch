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

package control

import (
	"context"
	"crypto/tls"
	"fmt"
	"io"
	"net"
	"sync"
	"time"

	quic "github.com/quic-go/quic-go"
	kcp "github.com/xtaci/kcp-go/v5"

	"github.com/miopunch/miopunch/internal/tlsutil"
)

type Protocol string

const (
	ProtoTCP  Protocol = "tcp"
	ProtoKCP  Protocol = "kcp"
	ProtoQUIC Protocol = "quic"
)

const (
	defaultALPN = "miopunch-xtcp-control"
)

type Listener interface {
	Accept(ctx context.Context) (io.ReadWriteCloser, error)
	Close() error
	Addr() net.Addr
}

func Listen(addr string, proto Protocol) (Listener, error) {
	switch proto {
	case ProtoTCP:
		l, err := net.Listen("tcp", addr)
		if err != nil {
			return nil, err
		}
		return &tcpListener{l: l}, nil
	case ProtoKCP:
		l, err := newKCPListener(addr)
		if err != nil {
			return nil, err
		}
		return l, nil
	case ProtoQUIC:
		tlsConfig, err := tlsutil.NewServerTLSConfig("", "", "")
		if err != nil {
			return nil, err
		}
		tlsConfig.NextProtos = []string{defaultALPN}
		l, err := quic.ListenAddr(addr, tlsConfig, &quic.Config{
			MaxIdleTimeout:  30 * time.Second,
			KeepAlivePeriod: 10 * time.Second,
		})
		if err != nil {
			return nil, err
		}
		return &quicListener{l: l}, nil
	default:
		return nil, fmt.Errorf("unknown control protocol: %q", proto)
	}
}

func Dial(ctx context.Context, addr string, proto Protocol) (io.ReadWriteCloser, error) {
	switch proto {
	case ProtoTCP:
		d := &net.Dialer{Timeout: 5 * time.Second}
		return d.DialContext(ctx, "tcp", addr)
	case ProtoKCP:
		sess, err := kcp.DialWithOptions(addr, nil, 10, 3)
		if err != nil {
			return nil, err
		}
		applyKCPSessionOptions(sess)
		return sess, nil
	case ProtoQUIC:
		tlsConfig, err := tlsutil.NewClientTLSConfig("", "", "", "")
		if err != nil {
			return nil, err
		}
		tlsConfig.NextProtos = []string{defaultALPN}

		c, err := quic.DialAddr(ctx, addr, tlsConfig, &quic.Config{
			MaxIdleTimeout:  30 * time.Second,
			KeepAlivePeriod: 10 * time.Second,
		})
		if err != nil {
			return nil, err
		}
		s, err := c.OpenStreamSync(ctx)
		if err != nil {
			_ = c.CloseWithError(0, "")
			return nil, err
		}
		return &quicStreamRWC{stream: s, conn: c}, nil
	default:
		return nil, fmt.Errorf("unknown control protocol: %q", proto)
	}
}

func applyKCPSessionOptions(conn *kcp.UDPSession) {
	_ = conn.SetReadBuffer(4 << 20)
	_ = conn.SetWriteBuffer(4 << 20)
	conn.SetStreamMode(true)
	conn.SetWriteDelay(false)
	conn.SetNoDelay(1, 20, 2, 1)
	conn.SetMtu(1350)
	conn.SetWindowSize(1024, 1024)
	conn.SetACKNoDelay(true)
}

type tcpListener struct{ l net.Listener }

func (l *tcpListener) Accept(_ context.Context) (io.ReadWriteCloser, error) { return l.l.Accept() }
func (l *tcpListener) Close() error                                         { return l.l.Close() }
func (l *tcpListener) Addr() net.Addr                                       { return l.l.Addr() }

type kcpListener struct {
	listener  *kcp.Listener
	acceptCh  chan net.Conn
	closeOnce sync.Once
	closeCh   chan struct{}
}

func newKCPListener(addr string) (*kcpListener, error) {
	listener, err := kcp.ListenWithOptions(addr, nil, 10, 3)
	if err != nil {
		return nil, err
	}
	_ = listener.SetReadBuffer(4 << 20)
	_ = listener.SetWriteBuffer(4 << 20)

	l := &kcpListener{
		listener: listener,
		acceptCh: make(chan net.Conn),
		closeCh:  make(chan struct{}),
	}
	go l.acceptLoop()
	return l, nil
}

func (l *kcpListener) acceptLoop() {
	for {
		sess, err := l.listener.AcceptKCP()
		if err != nil {
			select {
			case <-l.closeCh:
				close(l.acceptCh)
				return
			default:
			}
			continue
		}
		applyKCPSessionOptions(sess)
		select {
		case l.acceptCh <- sess:
		case <-l.closeCh:
			_ = sess.Close()
			close(l.acceptCh)
			return
		}
	}
}

func (l *kcpListener) Accept(ctx context.Context) (io.ReadWriteCloser, error) {
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case c, ok := <-l.acceptCh:
		if !ok {
			return nil, fmt.Errorf("kcp listener closed")
		}
		return c, nil
	}
}

func (l *kcpListener) Close() error {
	l.closeOnce.Do(func() {
		close(l.closeCh)
		_ = l.listener.Close()
	})
	return nil
}
func (l *kcpListener) Addr() net.Addr { return l.listener.Addr() }

type quicListener struct{ l *quic.Listener }

func (l *quicListener) Accept(ctx context.Context) (io.ReadWriteCloser, error) {
	c, err := l.l.Accept(ctx)
	if err != nil {
		return nil, err
	}
	s, err := c.AcceptStream(ctx)
	if err != nil {
		_ = c.CloseWithError(0, "")
		return nil, err
	}
	return &quicStreamRWC{stream: s, conn: c}, nil
}
func (l *quicListener) Close() error   { return l.l.Close() }
func (l *quicListener) Addr() net.Addr { return l.l.Addr() }

type quicStreamRWC struct {
	stream *quic.Stream
	conn   *quic.Conn
}

func (q *quicStreamRWC) Read(p []byte) (int, error)  { return q.stream.Read(p) }
func (q *quicStreamRWC) Write(p []byte) (int, error) { return q.stream.Write(p) }
func (q *quicStreamRWC) Close() error {
	q.stream.CancelRead(0)
	return q.stream.Close()
}

func ServerTLSConfig() (*tls.Config, error) {
	cfg, err := tlsutil.NewServerTLSConfig("", "", "")
	if err != nil {
		return nil, err
	}
	cfg.NextProtos = []string{defaultALPN}
	return cfg, nil
}
