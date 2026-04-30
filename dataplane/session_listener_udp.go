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
	"github.com/miopunch/miopunch/internal/logutil"
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
		tr := &quic.Transport{Conn: listenConn}
		ln, err := listenQUIC(ctx, cfg, tr, listenConn)
		if err != nil {
			_ = listenConn.Close()
			return nil, err
		}
		return &quicSessionListener{cfg: cfg, ln: ln, udp: listenConn, em: em}, nil
	case ProtocolKCP:
		ln, err := kcp.ServeConn(nil, 10, 3, listenConn)
		if err != nil {
			_ = listenConn.Close()
			return nil, err
		}
		return &kcpSessionListener{
			cfg: cfg,
			ln:  ln,
			pc:  listenConn,
			em:  em,
		}, nil
	default:
		_ = listenConn.Close()
		return nil, fmt.Errorf("listen does not support data proto: %q", cfg.Proto)
	}
}

// ListenSessionsWithQUICTransport is a QUIC-only helper for wiring an already
// constructed quic.Transport (socket owner) into a dataplane PeerSessionListener.
//
// The returned listener owns listenConn and will close it on Close().
func ListenSessionsWithQUICTransport(ctx context.Context, cfg Config, tr *quic.Transport, listenConn *net.UDPConn, em *event.Emitter) (PeerSessionListener, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	if cfg.Proto != ProtocolQUIC {
		return nil, fmt.Errorf("ListenSessionsWithQUICTransport requires proto=quic, got %q", cfg.Proto)
	}
	if listenConn == nil {
		return nil, errors.New("listen conn is required")
	}
	if tr == nil {
		_ = listenConn.Close()
		return nil, errors.New("nil quic transport")
	}
	if err := cfg.requirePinnedIdentity(); err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	if err := ctx.Err(); err != nil {
		_ = listenConn.Close()
		return nil, err
	}

	ln, err := listenQUIC(ctx, cfg, tr, listenConn)
	if err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	return &quicSessionListener{cfg: cfg, ln: ln, udp: listenConn, em: em}, nil
}

func listenQUIC(ctx context.Context, cfg Config, tr *quic.Transport, _ *net.UDPConn) (*quic.Listener, error) {
	_ = ctx

	tlsConfig, err := tlsutil.NewPinnedServerTLSConfig(cfg.SecretKey, cfg.SecurityID, tlsRoleClient, tlsRoleVisitor)
	if err != nil {
		return nil, err
	}
	tlsConfig.NextProtos = []string{dataALPN}

	return tr.Listen(tlsConfig, quicSessionConfig())
}

// ListenSessionsWithKCPPacketConn is a KCP-only helper for wiring an already
// demultiplexed packetconn (e.g. a socket owner wrapper) into a dataplane
// PeerSessionListener.
//
// The returned listener owns pc and will close it on Close().
func ListenSessionsWithKCPPacketConn(ctx context.Context, cfg Config, pc net.PacketConn, em *event.Emitter) (PeerSessionListener, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	if cfg.Proto != ProtocolKCP {
		return nil, fmt.Errorf("ListenSessionsWithKCPPacketConn requires proto=kcp, got %q", cfg.Proto)
	}
	if pc == nil {
		return nil, errors.New("packetconn is required")
	}
	if err := cfg.requirePinnedIdentity(); err != nil {
		_ = pc.Close()
		return nil, err
	}
	if err := ctx.Err(); err != nil {
		_ = pc.Close()
		return nil, err
	}

	ln, err := kcp.ServeConn(nil, 10, 3, pc)
	if err != nil {
		_ = pc.Close()
		return nil, err
	}
	return &kcpSessionListener{
		cfg: cfg,
		ln:  ln,
		pc:  pc,
		em:  em,
	}, nil
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
	pc  net.PacketConn
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
		logutil.Infof("kcp session accepted: sid=%s path_family=%s remote=%s conv=%d", l.cfg.SecurityID, l.cfg.PathFamily, sess.RemoteAddr(), sess.GetConv())

		applyKCPDefaults(sess)

		// KCP is an inner framing transport: we still bind identity via pinned TLS.
		tlsConn, err := pinnedTLSConn(sess, l.cfg, false)
		if err != nil {
			logutil.Infof("kcp tls setup error: sid=%s path_family=%s remote=%s err=%v", l.cfg.SecurityID, l.cfg.PathFamily, sess.RemoteAddr(), err)
			_ = sess.Close()
			continue
		}
		handshakeCtx, cancel := kcpAcceptHandshakeContext(ctx)
		err = tlsConn.HandshakeContext(handshakeCtx)
		cancel()
		if err != nil {
			logutil.Infof("kcp tls handshake error: sid=%s path_family=%s remote=%s err=%v", l.cfg.SecurityID, l.cfg.PathFamily, sess.RemoteAddr(), err)
			_ = tlsConn.Close()
			_ = sess.Close()
			if ctx.Err() != nil {
				return nil, ctx.Err()
			}
			continue
		}
		_ = tlsConn.SetDeadline(time.Time{})
		logutil.Infof("kcp tls handshake ok: sid=%s path_family=%s remote=%s", l.cfg.SecurityID, l.cfg.PathFamily, sess.RemoteAddr())

		owned := &ownedNetConn{
			Conn:    tlsConn,
			closers: []io.Closer{tlsConn, sess},
		}
		muxSession, err := yamuxForRole(owned, false)
		if err != nil {
			logutil.Infof("kcp yamux setup error: sid=%s path_family=%s remote=%s err=%v", l.cfg.SecurityID, l.cfg.PathFamily, sess.RemoteAddr(), err)
			_ = owned.Close()
			continue
		}
		logutil.Infof("kcp yamux session ready: sid=%s path_family=%s remote=%s", l.cfg.SecurityID, l.cfg.PathFamily, sess.RemoteAddr())
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
	if l.pc != nil {
		return l.pc.Close()
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
