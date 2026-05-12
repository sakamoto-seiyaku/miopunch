package dataplane

import (
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"io"
	"net"
	"sync"
	"time"

	"github.com/apernet/quic-go"
	"github.com/hashicorp/yamux"
	kcp "github.com/xtaci/kcp-go/v5"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/netutil"
	"github.com/miopunch/miopunch/internal/tlsutil"
)

type ownedNetConn struct {
	net.Conn
	closers []io.Closer
}

const kcpAcceptHandshakeTimeout = 5 * time.Second

func kcpAcceptHandshakeContext(ctx context.Context) (context.Context, context.CancelFunc) {
	return context.WithTimeout(ctx, kcpAcceptHandshakeTimeout)
}

func (c *ownedNetConn) Close() error {
	var firstErr error
	for _, closer := range c.closers {
		if closer == nil {
			continue
		}
		if err := closer.Close(); err != nil && firstErr == nil {
			firstErr = err
		}
	}
	return firstErr
}

type yamuxPeerSession struct {
	sessionBase
	sess *yamux.Session
}

func newYamuxPeerSession(key SessionKey, sess *yamux.Session, em *event.Emitter, impl string, idleTimeout time.Duration) *yamuxPeerSession {
	s := &yamuxPeerSession{
		sessionBase: newSessionBase(key, em),
		sess:        sess,
	}
	emitSessionOpen(em, s.Key(), impl)
	s.startIdleCloser(idleTimeout)
	return s
}

func (s *yamuxPeerSession) OpenStream(ctx context.Context, open StreamOpen) (io.ReadWriteCloser, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if !s.Healthy() {
		return nil, io.ErrClosedPipe
	}
	stream, err := s.sess.OpenStream()
	if err != nil {
		_ = s.Close(CloseReasonTransportFatal)
		return nil, err
	}
	if err := WriteStreamOpen(stream, open); err != nil {
		_ = stream.Close()
		_ = s.Close(CloseReasonStreamProtocolError)
		return nil, err
	}
	s.markActivity()
	emitLogicalStream(s.em, s.Key(), open, false)
	return &logicalStream{
		rwc:        stream,
		onActivity: s.markActivity,
		em:         s.em,
		key:        s.Key(),
		open:       open,
	}, nil
}

func (s *yamuxPeerSession) AcceptStream(ctx context.Context) (*AcceptedStream, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if !s.Healthy() {
		return nil, io.ErrClosedPipe
	}

	stream, err := s.sess.AcceptStreamWithContext(ctx)
	if err != nil {
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		_ = s.Close(CloseReasonTransportFatal)
		return nil, err
	}
	open, err := ReadStreamOpen(stream)
	if err != nil {
		_ = stream.Close()
		_ = s.Close(CloseReasonStreamProtocolError)
		return nil, err
	}
	s.markActivity()
	emitLogicalStream(s.em, s.Key(), open, true)
	return &AcceptedStream{
		Stream: &logicalStream{
			rwc:        stream,
			onActivity: s.markActivity,
			em:         s.em,
			key:        s.Key(),
			open:       open,
			accept:     true,
		},
		Open: open,
	}, nil
}

func (s *yamuxPeerSession) Close(reason CloseReason) error {
	return s.closeBase(reason, s.sess.Close)
}

func (s *yamuxPeerSession) startIdleCloser(timeout time.Duration) {
	if timeout <= 0 {
		timeout = DefaultSessionIdleTimeout
	}
	interval := timeout / 2
	if interval < time.Second {
		interval = time.Second
	}

	go func() {
		ticker := time.NewTicker(interval)
		defer ticker.Stop()
		for {
			select {
			case <-s.done:
				return
			case <-ticker.C:
				if s.idleFor() >= timeout {
					_ = s.Close(CloseReasonIdleTimeout)
					return
				}
			}
		}
	}()
}

type quicPeerSession struct {
	sessionBase
	conn *quic.Conn
	udp  *net.UDPConn
	ln   *quic.Listener
}

func newQUICPeerSession(key SessionKey, conn *quic.Conn, udp *net.UDPConn, ln *quic.Listener, em *event.Emitter, idleTimeout time.Duration) *quicPeerSession {
	s := &quicPeerSession{
		sessionBase: newSessionBase(key, em),
		conn:        conn,
		udp:         udp,
		ln:          ln,
	}
	emitSessionOpen(em, s.Key(), "quic-go")
	s.startIdleCloser(idleTimeout)
	return s
}

func (s *quicPeerSession) OpenStream(ctx context.Context, open StreamOpen) (io.ReadWriteCloser, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if !s.Healthy() {
		return nil, io.ErrClosedPipe
	}
	stream, err := s.conn.OpenStreamSync(ctx)
	if err != nil {
		_ = s.Close(CloseReasonTransportFatal)
		return nil, err
	}
	if err := WriteStreamOpen(stream, open); err != nil {
		_ = stream.Close()
		_ = s.Close(CloseReasonStreamProtocolError)
		return nil, err
	}
	s.markActivity()
	emitLogicalStream(s.em, s.Key(), open, false)
	return &logicalStream{
		rwc:        stream,
		onActivity: s.markActivity,
		em:         s.em,
		key:        s.Key(),
		open:       open,
	}, nil
}

func (s *quicPeerSession) AcceptStream(ctx context.Context) (*AcceptedStream, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if !s.Healthy() {
		return nil, io.ErrClosedPipe
	}
	stream, err := s.conn.AcceptStream(ctx)
	if err != nil {
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		_ = s.Close(CloseReasonTransportFatal)
		return nil, err
	}
	open, err := ReadStreamOpen(stream)
	if err != nil {
		_ = stream.Close()
		_ = s.Close(CloseReasonStreamProtocolError)
		return nil, err
	}
	s.markActivity()
	emitLogicalStream(s.em, s.Key(), open, true)
	return &AcceptedStream{
		Stream: &logicalStream{
			rwc:        stream,
			onActivity: s.markActivity,
			em:         s.em,
			key:        s.Key(),
			open:       open,
			accept:     true,
		},
		Open: open,
	}, nil
}

func (s *quicPeerSession) Close(reason CloseReason) error {
	return s.closeBase(reason, func() error {
		if s.conn != nil {
			s.conn.CloseWithError(0, string(reason))
		}
		if s.ln != nil {
			_ = s.ln.Close()
		}
		if s.udp != nil {
			_ = s.udp.Close()
		}
		return nil
	})
}

func (s *quicPeerSession) startIdleCloser(timeout time.Duration) {
	if timeout <= 0 {
		timeout = DefaultSessionIdleTimeout
	}
	interval := timeout / 2
	if interval < time.Second {
		interval = time.Second
	}

	go func() {
		ticker := time.NewTicker(interval)
		defer ticker.Stop()
		for {
			select {
			case <-s.done:
				return
			case <-s.conn.Context().Done():
				_ = s.Close(CloseReasonTransportFatal)
				return
			case <-ticker.C:
				if s.idleFor() >= timeout {
					_ = s.Close(CloseReasonIdleTimeout)
					return
				}
			}
		}
	}()
}

// DialSession establishes a KCP or QUIC peer transport session.
func DialSession(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, em *event.Emitter) (PeerSession, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	switch cfg.Proto {
	case ProtocolKCP:
		return dialKCPSession(ctx, cfg, listenConn, raddr, em)
	case ProtocolQUIC:
		return dialQUICSession(ctx, cfg, listenConn, raddr, em)
	default:
		return nil, fmt.Errorf("session dial does not support data proto: %q", cfg.Proto)
	}
}

// DialSessionWithQUICTransport establishes a QUIC peer transport session using an
// already-constructed quic.Transport (UDP socket owner).
//
// The returned session does NOT close the underlying UDP socket on Close(). The
// caller owns the transport / UDPConn lifecycle.
func DialSessionWithQUICTransport(ctx context.Context, cfg Config, tr *quic.Transport, raddr *net.UDPAddr, em *event.Emitter) (PeerSession, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	if cfg.Proto != ProtocolQUIC {
		return nil, fmt.Errorf("DialSessionWithQUICTransport requires proto=quic, got %q", cfg.Proto)
	}
	if tr == nil {
		return nil, errors.New("nil quic transport")
	}
	if err := cfg.requirePinnedIdentity(); err != nil {
		return nil, err
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if raddr == nil {
		return nil, errors.New("quic requires remote addr")
	}

	tlsConfig, err := tlsutil.NewPinnedClientTLSConfig(cfg.SecretKey, cfg.SecurityID, tlsRoleVisitor, tlsRoleClient)
	if err != nil {
		return nil, err
	}
	tlsConfig.NextProtos = []string{dataALPN}

	conn, err := tr.Dial(ctx, raddr, tlsConfig, quicSessionConfig())
	if err != nil {
		return nil, err
	}
	if err := applyQUICCC(cfg, conn); err != nil {
		conn.CloseWithError(0, "")
		return nil, err
	}
	return newQUICPeerSession(cfg.sessionKey(), conn, nil, nil, em, cfg.IdleTimeout), nil
}

// DialSessionWithKCPPacketConn establishes a KCP peer transport session using an
// already-constructed net.PacketConn (typically a socket-owner wrapper).
//
// The returned session does NOT close the underlying packetconn on Close(). The
// caller owns the packetconn lifecycle.
func DialSessionWithKCPPacketConn(ctx context.Context, cfg Config, pc net.PacketConn, raddr *net.UDPAddr, em *event.Emitter) (PeerSession, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	if cfg.Proto != ProtocolKCP {
		return nil, fmt.Errorf("DialSessionWithKCPPacketConn requires proto=kcp, got %q", cfg.Proto)
	}
	if pc == nil {
		return nil, errors.New("nil packetconn")
	}
	if err := cfg.requirePinnedIdentity(); err != nil {
		return nil, err
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if raddr == nil {
		return nil, errors.New("kcp requires remote addr")
	}

	clearPacketConnDeadline := func() {}
	if deadline, ok := ctx.Deadline(); ok {
		_ = pc.SetDeadline(deadline)
		clearPacketConnDeadline = func() { _ = pc.SetDeadline(time.Time{}) }
		// pc may be backed by a long-lived UDP socket owner. Ensure we don't leak
		// the handshake deadline to the established session.
		defer clearPacketConnDeadline()
	}

	kcpConn, err := netutil.NewKCPConnFromPacketConn(pc, raddr.String())
	if err != nil {
		return nil, err
	}
	if oob, ok := kcpConn.(interface{ SendOOB([]byte) error }); ok {
		_ = oob.SendOOB([]byte{0})
	}
	if deadline, ok := ctx.Deadline(); ok {
		_ = kcpConn.SetDeadline(deadline)
	}

	tlsConn, err := pinnedTLSConn(kcpConn, cfg, true)
	if err != nil {
		_ = kcpConn.Close()
		return nil, err
	}
	if err := tlsConn.HandshakeContext(ctx); err != nil {
		_ = tlsConn.Close()
		_ = kcpConn.Close()
		return nil, err
	}
	_ = tlsConn.SetDeadline(time.Time{})
	clearPacketConnDeadline()
	_ = kcpConn.SetDeadline(time.Time{})

	owned := &ownedNetConn{
		Conn:    tlsConn,
		closers: []io.Closer{tlsConn, kcpConn},
	}
	muxSession, err := yamuxForRole(owned, true)
	if err != nil {
		_ = owned.Close()
		return nil, err
	}
	return newYamuxPeerSession(cfg.sessionKey(), muxSession, em, "kcp+tls+yamux", cfg.IdleTimeout), nil
}

// ServeSession accepts a KCP or QUIC peer transport session.
func ServeSession(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, em *event.Emitter) (PeerSession, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	switch cfg.Proto {
	case ProtocolKCP:
		return serveKCPSession(ctx, cfg, listenConn, raddr, em)
	case ProtocolQUIC:
		return serveQUICSession(ctx, cfg, listenConn, em)
	default:
		return nil, fmt.Errorf("session serve does not support data proto: %q", cfg.Proto)
	}
}

func dialKCPSession(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, em *event.Emitter) (PeerSession, error) {
	return newKCPSession(ctx, cfg, listenConn, raddr, true, em)
}

func serveKCPSession(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, em *event.Emitter) (PeerSession, error) {
	_ = raddr
	if listenConn == nil {
		return nil, errors.New("kcp requires listen conn")
	}
	if err := cfg.requirePinnedIdentity(); err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	if err := ctx.Err(); err != nil {
		_ = listenConn.Close()
		return nil, err
	}

	ln, err := kcp.ServeConn(nil, 10, 3, listenConn)
	if err != nil {
		_ = listenConn.Close()
		return nil, err
	}

	for {
		if err := ctx.Err(); err != nil {
			_ = ln.Close()
			_ = listenConn.Close()
			return nil, err
		}

		// AcceptKCP doesn't take a context. Use a short deadline to poll.
		_ = ln.SetDeadline(time.Now().Add(250 * time.Millisecond))
		kcpSess, err := ln.AcceptKCP()
		if err != nil {
			var ne net.Error
			if errors.As(err, &ne) && ne.Timeout() {
				continue
			}
			_ = ln.Close()
			_ = listenConn.Close()
			if ctx.Err() != nil {
				return nil, ctx.Err()
			}
			return nil, err
		}
		_ = ln.SetDeadline(time.Time{})

		applyKCPDefaults(kcpSess)

		// KCP is an inner framing transport: we still bind identity via pinned TLS.
		tlsConn, err := pinnedTLSConn(kcpSess, cfg, false)
		if err != nil {
			_ = kcpSess.Close()
			continue
		}
		handshakeCtx, cancel := kcpAcceptHandshakeContext(ctx)
		err = tlsConn.HandshakeContext(handshakeCtx)
		cancel()
		if err != nil {
			_ = tlsConn.Close()
			_ = kcpSess.Close()
			if ctx.Err() != nil {
				_ = ln.Close()
				_ = listenConn.Close()
				return nil, ctx.Err()
			}
			continue
		}
		_ = tlsConn.SetDeadline(time.Time{})

		owned := &ownedNetConn{
			Conn:    tlsConn,
			closers: []io.Closer{tlsConn, kcpSess, ln, listenConn},
		}
		muxSession, err := yamuxForRole(owned, false)
		if err != nil {
			_ = owned.Close()
			continue
		}
		return newYamuxPeerSession(cfg.sessionKey(), muxSession, em, "kcp+tls+yamux", cfg.IdleTimeout), nil
	}
}

func newKCPSession(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, asClient bool, em *event.Emitter) (PeerSession, error) {
	if listenConn == nil {
		return nil, errors.New("kcp requires listen conn")
	}
	if err := cfg.requirePinnedIdentity(); err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	if err := ctx.Err(); err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	if raddr == nil {
		_ = listenConn.Close()
		return nil, errors.New("kcp requires remote addr")
	}

	if deadline, ok := ctx.Deadline(); ok {
		if err := listenConn.SetDeadline(deadline); err != nil {
			_ = listenConn.Close()
			return nil, err
		}
	}

	kcpConn, err := netutil.NewKCPConnFromUDP(listenConn, false, raddr.String())
	if err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	// Best-effort "priming" packet:
	// - KCP has no SYN. A server-side kcp.Listener only starts tracking a remote
	//   once it sees any packet from it.
	// - Sending a small OOB packet (bypasses the reliable stream) helps ensure
	//   the listener Accept path can return promptly before TLS/yamux start.
	if oob, ok := kcpConn.(interface{ SendOOB([]byte) error }); ok {
		_ = oob.SendOOB([]byte{0})
	}
	if deadline, ok := ctx.Deadline(); ok {
		if err := kcpConn.SetDeadline(deadline); err != nil {
			_ = kcpConn.Close()
			_ = listenConn.Close()
			return nil, err
		}
	}

	tlsConn, err := pinnedTLSConn(kcpConn, cfg, asClient)
	if err != nil {
		_ = kcpConn.Close()
		_ = listenConn.Close()
		return nil, err
	}
	if err := tlsConn.HandshakeContext(ctx); err != nil {
		_ = tlsConn.Close()
		_ = kcpConn.Close()
		_ = listenConn.Close()
		return nil, err
	}
	_ = tlsConn.SetDeadline(time.Time{})
	_ = listenConn.SetDeadline(time.Time{})

	owned := &ownedNetConn{
		Conn:    tlsConn,
		closers: []io.Closer{tlsConn, listenConn},
	}
	muxSession, err := yamuxForRole(owned, asClient)
	if err != nil {
		_ = owned.Close()
		return nil, err
	}
	return newYamuxPeerSession(cfg.sessionKey(), muxSession, em, "kcp+tls+yamux", cfg.IdleTimeout), nil
}

func pinnedTLSConn(conn net.Conn, cfg Config, asClient bool) (*tls.Conn, error) {
	var tlsConfig *tls.Config
	var err error
	if asClient {
		tlsConfig, err = tlsutil.NewPinnedClientTLSConfig(cfg.SecretKey, cfg.SecurityID, tlsRoleVisitor, tlsRoleClient)
	} else {
		tlsConfig, err = tlsutil.NewPinnedServerTLSConfig(cfg.SecretKey, cfg.SecurityID, tlsRoleClient, tlsRoleVisitor)
	}
	if err != nil {
		return nil, err
	}
	tlsConfig.NextProtos = []string{dataALPN}
	if asClient {
		return tls.Client(conn, tlsConfig), nil
	}
	return tls.Server(conn, tlsConfig), nil
}

func yamuxForRole(conn io.ReadWriteCloser, asClient bool) (*yamux.Session, error) {
	cfg := yamux.DefaultConfig()
	cfg.LogOutput = io.Discard
	cfg.MaxStreamWindowSize = 6 * 1024 * 1024
	if asClient {
		return yamux.Client(conn, cfg)
	}
	return yamux.Server(conn, cfg)
}

func dialQUICSession(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, em *event.Emitter) (PeerSession, error) {
	if listenConn == nil {
		return nil, errors.New("quic requires listen conn")
	}
	if err := cfg.requirePinnedIdentity(); err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	if raddr == nil {
		_ = listenConn.Close()
		return nil, errors.New("quic requires remote addr")
	}

	tlsConfig, err := tlsutil.NewPinnedClientTLSConfig(cfg.SecretKey, cfg.SecurityID, tlsRoleVisitor, tlsRoleClient)
	if err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	tlsConfig.NextProtos = []string{dataALPN}

	tr := &quic.Transport{Conn: listenConn}
	conn, err := tr.Dial(ctx, raddr, tlsConfig, quicSessionConfig())
	if err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	if err := applyQUICCC(cfg, conn); err != nil {
		conn.CloseWithError(0, "")
		_ = listenConn.Close()
		return nil, err
	}
	return newQUICPeerSession(cfg.sessionKey(), conn, listenConn, nil, em, cfg.IdleTimeout), nil
}

func serveQUICSession(ctx context.Context, cfg Config, listenConn *net.UDPConn, em *event.Emitter) (PeerSession, error) {
	if listenConn == nil {
		return nil, errors.New("quic requires listen conn")
	}
	if err := cfg.requirePinnedIdentity(); err != nil {
		_ = listenConn.Close()
		return nil, err
	}

	tlsConfig, err := tlsutil.NewPinnedServerTLSConfig(cfg.SecretKey, cfg.SecurityID, tlsRoleClient, tlsRoleVisitor)
	if err != nil {
		_ = listenConn.Close()
		return nil, err
	}
	tlsConfig.NextProtos = []string{dataALPN}

	tr := &quic.Transport{Conn: listenConn}
	ln, err := tr.Listen(tlsConfig, quicSessionConfig())
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
	return newQUICPeerSession(cfg.sessionKey(), conn, listenConn, ln, em, cfg.IdleTimeout), nil
}

func quicSessionConfig() *quic.Config {
	return &quic.Config{
		HandshakeIdleTimeout: 20 * time.Second,
		MaxIdleTimeout:       30 * time.Second,
		KeepAlivePeriod:      10 * time.Second,
	}
}

// DialTLSSession establishes a TCP pinned TLS plus yamux peer session.
func DialTLSSession(ctx context.Context, cfg Config, candidates []connectivity.TCPConn, em *event.Emitter) (PeerSession, error) {
	return newTLSSession(ctx, cfg, candidates, true, em)
}

// ServeTLSSession accepts a TCP pinned TLS plus yamux peer session.
func ServeTLSSession(ctx context.Context, cfg Config, candidates []connectivity.TCPConn, em *event.Emitter) (PeerSession, error) {
	return newTLSSession(ctx, cfg, candidates, false, em)
}

func newTLSSession(ctx context.Context, cfg Config, candidates []connectivity.TCPConn, asClient bool, em *event.Emitter) (PeerSession, error) {
	cfg.Proto = ProtocolTLS
	cfg.Normalize()
	if err := cfg.requirePinnedIdentity(); err != nil {
		closeTCPCandidates(candidates)
		return nil, err
	}

	selfRole := tlsRoleClient
	peerRole := tlsRoleVisitor
	if asClient {
		selfRole = tlsRoleVisitor
		peerRole = tlsRoleClient
	}
	tlsConn, err := convergePinnedTLS(ctx, cfg.SecurityID, cfg.SecretKey, selfRole, peerRole, asClient, candidates, em)
	if err != nil {
		return nil, err
	}
	muxSession, err := yamuxForRole(tlsConn, asClient)
	if err != nil {
		_ = tlsConn.Close()
		return nil, err
	}
	return newYamuxPeerSession(cfg.sessionKey(), muxSession, em, "tls+yamux", cfg.IdleTimeout), nil
}

type sessionOwnedStream struct {
	io.ReadWriteCloser
	session PeerSession
	once    sync.Once
}

func (s *sessionOwnedStream) Close() error {
	var err error
	s.once.Do(func() {
		err = s.ReadWriteCloser.Close()
		if closeErr := s.session.Close(CloseReasonDaemonShutdown); closeErr != nil && err == nil {
			err = closeErr
		}
	})
	return err
}

func (s *sessionOwnedStream) SetDeadline(t time.Time) error {
	conn, ok := s.ReadWriteCloser.(interface{ SetDeadline(time.Time) error })
	if !ok {
		return nil
	}
	return conn.SetDeadline(t)
}

func (s *sessionOwnedStream) SetReadDeadline(t time.Time) error {
	conn, ok := s.ReadWriteCloser.(interface{ SetReadDeadline(time.Time) error })
	if !ok {
		return nil
	}
	return conn.SetReadDeadline(t)
}

func (s *sessionOwnedStream) SetWriteDeadline(t time.Time) error {
	conn, ok := s.ReadWriteCloser.(interface{ SetWriteDeadline(time.Time) error })
	if !ok {
		return nil
	}
	return conn.SetWriteDeadline(t)
}
