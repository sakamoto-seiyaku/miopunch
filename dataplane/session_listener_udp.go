package dataplane

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"strings"
	"time"

	"github.com/apernet/quic-go"
	kcp "github.com/xtaci/kcp-go/v5"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/tlsutil"
)

// ListenSessions creates a server-side listener for inbound peer sessions over
// a UDP socket (QUIC or KCP).
//
// The returned listener owns listenConn and will close it on Close().
func ListenSessions(ctx context.Context, cfg Config, listenConn *net.UDPConn, em *event.Emitter) (PeerSessionListener, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	if listenConn == nil {
		return nil, errors.New("listen conn is required")
	}
	if err := cfg.requirePinnedIdentity(); err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	if err := ctx.Err(); err != nil {
		_ = listenConn.Close()
		return nil, err
	}

	switch cfg.Proto {
	case ProtocolQUIC:
		tlsConfig, err := tlsutil.NewPinnedServerTLSConfig(cfg.SecretKey, cfg.SecurityID, tlsRoleClient, tlsRoleVisitor)
		if err != nil {
			_ = listenConn.Close()
			return nil, err
		}
		tlsConfig.NextProtos = []string{dataALPN}

		ln, err := quic.Listen(listenConn, tlsConfig, quicSessionConfig())
		if err != nil {
			_ = listenConn.Close()
			return nil, err
		}
		return &quicSessionListener{
			cfg: cfg,
			ln:  ln,
			udp: listenConn,
			em:  em,
		}, nil
	case ProtocolKCP:
		ln, err := kcp.ServeConn(nil, 10, 3, listenConn)
		if err != nil {
			_ = listenConn.Close()
			return nil, err
		}
		return &kcpSessionListener{
			cfg: cfg,
			ln:  ln,
			udp: listenConn,
			em:  em,
		}, nil
	default:
		_ = listenConn.Close()
		return nil, fmt.Errorf("listen does not support data proto: %q", cfg.Proto)
	}
}

type quicSessionListener struct {
	cfg Config
	ln  *quic.Listener
	udp *net.UDPConn
	em  *event.Emitter
}

func (l *quicSessionListener) Accept(ctx context.Context) (PeerSession, error) {
	if ctx.Err() != nil {
		return nil, ctx.Err()
	}
	conn, err := l.ln.Accept(ctx)
	if err != nil {
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		return nil, err
	}
	if err := applyQUICCC(l.cfg, conn); err != nil {
		conn.CloseWithError(0, "")
		return nil, err
	}
	// Accepted connections share the underlying UDP socket owned by the listener.
	return newQUICPeerSession(l.cfg.sessionKey(), conn, nil, nil, l.em, l.cfg.IdleTimeout), nil
}

func (l *quicSessionListener) Close() error {
	if l == nil {
		return nil
	}
	if l.ln != nil {
		_ = l.ln.Close()
	}
	if l.udp != nil {
		return l.udp.Close()
	}
	return nil
}

type kcpSessionListener struct {
	cfg Config
	ln  *kcp.Listener
	udp *net.UDPConn
	em  *event.Emitter
}

func (l *kcpSessionListener) Accept(ctx context.Context) (PeerSession, error) {
	if ctx.Err() != nil {
		return nil, ctx.Err()
	}
	for {
		if err := ctx.Err(); err != nil {
			return nil, err
		}

		// AcceptKCP doesn't take a context. Use a short read deadline to poll.
		_ = l.ln.SetDeadline(time.Now().Add(250 * time.Millisecond))
		sess, err := l.ln.AcceptKCP()
		if err != nil {
			var ne net.Error
			// kcp-go wraps its timeout sentinel (timeoutError) with pkg/errors, so
			// it may not always present as a net.Error. Treat any timeout-ish error
			// as a poll signal.
			if (errors.As(err, &ne) && ne.Timeout()) || strings.Contains(err.Error(), "timeout") {
				continue
			}
			if ctx.Err() != nil {
				return nil, ctx.Err()
			}
			return nil, err
		}
		_ = l.ln.SetDeadline(time.Time{})

		// Our dial path currently uses a fixed conv (see internal/netutil). The
		// listener can accept arbitrary conv values from any UDP packet, so filter
		// aggressively to avoid mistaking late punching traffic for a KCP session.
		if sess.GetConv() != 1 {
			_ = sess.Close()
			continue
		}

		applyKCPDefaults(sess)

		// KCP is an inner framing transport: we still bind identity via pinned TLS.
		tlsConn, err := pinnedTLSConn(sess, l.cfg, false)
		if err != nil {
			_ = sess.Close()
			continue
		}
		if err := tlsConn.HandshakeContext(ctx); err != nil {
			_ = tlsConn.Close()
			_ = sess.Close()
			continue
		}
		_ = tlsConn.SetDeadline(time.Time{})

		owned := &ownedNetConn{
			Conn:    tlsConn,
			closers: []io.Closer{tlsConn, sess},
		}
		muxSession, err := yamuxForRole(owned, false)
		if err != nil {
			_ = owned.Close()
			continue
		}
		return newYamuxPeerSession(l.cfg.sessionKey(), muxSession, l.em, "kcp+tls+yamux", l.cfg.IdleTimeout), nil
	}
}

func (l *kcpSessionListener) Close() error {
	if l == nil {
		return nil
	}
	if l.ln != nil {
		_ = l.ln.Close()
	}
	if l.udp != nil {
		return l.udp.Close()
	}
	return nil
}

func applyKCPDefaults(sess *kcp.UDPSession) {
	if sess == nil {
		return
	}
	sess.SetStreamMode(true)
	sess.SetWriteDelay(true)
	sess.SetNoDelay(1, 20, 2, 1)
	sess.SetMtu(1350)
	sess.SetWindowSize(1024, 1024)
	sess.SetACKNoDelay(false)
}
