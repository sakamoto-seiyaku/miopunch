package dataplane

import (
	"context"
	"errors"
	"fmt"
	"net"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/event"
)

// ListenTLSSessions accepts inbound peer sessions over TCP using pinned TLS plus yamux.
//
// The returned listener owns ln and will close it on Close().
func ListenTLSSessions(ctx context.Context, cfg Config, ln *net.TCPListener, em *event.Emitter) (PeerSessionListener, error) {
	cfg.Proto = ProtocolTLS
	cfg.Normalize()
	if err := cfg.requirePinnedIdentity(); err != nil {
		if ln != nil {
			_ = ln.Close()
		}
		return nil, err
	}
	if ln == nil {
		return nil, errors.New("tcp listener is required")
	}
	if err := ctx.Err(); err != nil {
		_ = ln.Close()
		return nil, err
	}
	return &tcpTLSSessionListener{
		cfg: cfg,
		ln:  ln,
		em:  em,
	}, nil
}

type tcpTLSSessionListener struct {
	cfg Config
	ln  *net.TCPListener
	em  *event.Emitter
}

func (l *tcpTLSSessionListener) Accept(ctx context.Context) (PeerSession, error) {
	if ctx.Err() != nil {
		return nil, ctx.Err()
	}
	for {
		if err := ctx.Err(); err != nil {
			return nil, err
		}

		// net.Listener.Accept() doesn't take a context. Poll with deadlines.
		_ = l.ln.SetDeadline(time.Now().Add(250 * time.Millisecond))
		conn, err := l.ln.Accept()
		if err != nil {
			var ne net.Error
			if errors.As(err, &ne) && ne.Timeout() {
				continue
			}
			if ctx.Err() != nil {
				return nil, ctx.Err()
			}
			return nil, err
		}
		_ = l.ln.SetDeadline(time.Time{})

		return ServeTLSSession(ctx, l.cfg, []connectivity.TCPConn{
			{Conn: conn, Origin: connectivity.TCPConnOriginAccept},
		}, l.em)
	}
}

func (l *tcpTLSSessionListener) Close() error {
	if l == nil || l.ln == nil {
		return nil
	}
	return l.ln.Close()
}

func (l *tcpTLSSessionListener) String() string {
	if l == nil || l.ln == nil {
		return "tls(tcp):<nil>"
	}
	return fmt.Sprintf("tls(tcp):%s", l.ln.Addr().String())
}
