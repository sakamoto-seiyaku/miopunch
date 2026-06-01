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

package session

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"fmt"
	"io"
	"net"
	"sync"
	"time"

	"github.com/hashicorp/yamux"
	kcp "github.com/xtaci/kcp-go/v5"

	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/netutil"
	"github.com/miopunch/miopunch/internal/pocv1/enroll"
	"github.com/miopunch/miopunch/internal/pocv1/punch"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
	"github.com/miopunch/miopunch/internal/tlsutil"
)

const sessionALPN = "miopunch-session-v1"

type peerSession struct {
	mu           sync.Mutex
	key          dataplane.SessionKey
	sess         *yamux.Session
	transport    net.Conn
	pathFacts    dataplane.SessionPathFacts
	lastActivity time.Time
	closeReason  dataplane.CloseReason
	closed       bool
	done         chan struct{}
	localOpen    StreamOpen
	acceptOpen   StreamOpen
}

type ownedConn struct {
	net.Conn
	closers []io.Closer
}

func (c *ownedConn) Close() error {
	var firstErr error
	for _, closer := range c.closers {
		if closer == nil {
			continue
		}
		if err := closer.Close(); err != nil && !errors.Is(err, net.ErrClosed) && !errors.Is(err, io.ErrClosedPipe) && firstErr == nil {
			firstErr = err
		}
	}
	return firstErr
}

func (s *peerSession) Key() dataplane.SessionKey {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.key
}

func (s *peerSession) SessionPathFacts() dataplane.SessionPathFacts {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.pathFacts.Normalize()
}

func (s *peerSession) OpenStream(ctx context.Context, open StreamOpen) (io.ReadWriteCloser, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if !s.Healthy() {
		return nil, io.ErrClosedPipe
	}
	stream, err := s.sess.OpenStream()
	if err != nil {
		_ = s.Close(dataplane.CloseReasonTransportFatal)
		return nil, err
	}
	if err := writeStreamOpenWithContext(ctx, stream, open); err != nil {
		_ = stream.Close()
		if ctxErr := ctx.Err(); ctxErr != nil && errors.Is(err, ctxErr) {
			return nil, err
		}
		_ = s.Close(dataplane.CloseReasonStreamProtocolError)
		return nil, err
	}
	s.markActivity()
	s.setLocalOpen(open)
	return &sessionStream{ReadWriteCloser: stream, session: s, open: open}, nil
}

func (s *peerSession) AcceptStream(ctx context.Context) (*dataplane.AcceptedStream, error) {
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
		_ = s.Close(dataplane.CloseReasonTransportFatal)
		return nil, err
	}
	open, err := readStreamOpenWithContext(ctx, stream)
	if err != nil {
		_ = stream.Close()
		if ctxErr := ctx.Err(); ctxErr != nil && errors.Is(err, ctxErr) {
			return nil, err
		}
		_ = s.Close(dataplane.CloseReasonStreamProtocolError)
		return nil, err
	}
	s.markActivity()
	s.setAcceptOpen(open)
	return &dataplane.AcceptedStream{
		Stream: &sessionStream{ReadWriteCloser: stream, session: s, open: open, accept: true},
		Open:   open,
	}, nil
}

func (s *peerSession) Close(reason dataplane.CloseReason) error {
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		return nil
	}
	s.closed = true
	s.closeReason = normalizeCloseReason(reason)
	done := s.done
	s.mu.Unlock()

	close(done)
	var firstErr error
	if s.sess != nil {
		if err := s.sess.Close(); err != nil && firstErr == nil {
			firstErr = err
		}
	}
	if s.transport != nil {
		if err := s.transport.Close(); err != nil && firstErr == nil {
			firstErr = err
		}
	}
	return firstErr
}

func (s *peerSession) CloseReason() dataplane.CloseReason {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.closeReason
}

func (s *peerSession) Healthy() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return !s.closed
}

func (s *peerSession) LastActivity() time.Time {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.lastActivity
}

func (s *peerSession) markActivity() {
	s.mu.Lock()
	s.lastActivity = time.Now()
	s.mu.Unlock()
}

func (s *peerSession) setLocalOpen(open StreamOpen) {
	s.mu.Lock()
	s.localOpen = open
	s.mu.Unlock()
}

func (s *peerSession) setAcceptOpen(open StreamOpen) {
	s.mu.Lock()
	s.acceptOpen = open
	s.mu.Unlock()
}

func (s *peerSession) LocalOpen() StreamOpen {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.localOpen
}

func (s *peerSession) AcceptOpen() StreamOpen {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.acceptOpen
}

func (s *peerSession) startIdleCloser(timeout time.Duration) {
	if timeout <= 0 {
		timeout = dataplane.DefaultSessionIdleTimeout
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
			case <-s.sess.CloseChan():
				_ = s.Close(dataplane.CloseReasonTransportFatal)
				return
			case <-ticker.C:
				if time.Since(s.LastActivity()) >= timeout {
					_ = s.Close(dataplane.CloseReasonIdleTimeout)
					return
				}
			}
		}
	}()
}

type sessionStream struct {
	io.ReadWriteCloser
	session *peerSession
	open    StreamOpen
	accept  bool
	once    sync.Once
}

func (s *sessionStream) Read(p []byte) (int, error) {
	n, err := s.ReadWriteCloser.Read(p)
	if n > 0 {
		s.session.markActivity()
	}
	return n, err
}

func (s *sessionStream) Write(p []byte) (int, error) {
	n, err := s.ReadWriteCloser.Write(p)
	if n > 0 {
		s.session.markActivity()
	}
	return n, err
}

func (s *sessionStream) Close() error {
	var err error
	s.once.Do(func() {
		err = s.ReadWriteCloser.Close()
		s.session.markActivity()
	})
	return err
}

func (s *sessionStream) SetDeadline(t time.Time) error {
	type deadlineSetter interface {
		SetDeadline(time.Time) error
	}
	if conn, ok := s.ReadWriteCloser.(deadlineSetter); ok {
		return conn.SetDeadline(t)
	}
	return nil
}

func (s *sessionStream) SetReadDeadline(t time.Time) error {
	type deadlineSetter interface {
		SetReadDeadline(time.Time) error
	}
	if conn, ok := s.ReadWriteCloser.(deadlineSetter); ok {
		return conn.SetReadDeadline(t)
	}
	return nil
}

func (s *sessionStream) SetWriteDeadline(t time.Time) error {
	type deadlineSetter interface {
		SetWriteDeadline(time.Time) error
	}
	if conn, ok := s.ReadWriteCloser.(deadlineSetter); ok {
		return conn.SetWriteDeadline(t)
	}
	return nil
}

// Dial upgrades the supplied PathResult into a live outbound peer session.
func upgrade(ctx context.Context, cfg Config, result punch.PathResult, asClient bool) (PeerSession, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	networkID, err := wire.CanonicalizeNetworkID(cfg.NetworkID)
	if err != nil {
		return nil, fmt.Errorf("canonicalize network id: %w", err)
	}
	if result.Conn == nil {
		return nil, errors.New("path result connection is required")
	}
	if result.RemoteAddr == nil {
		return nil, errors.New("path result remote address is required")
	}
	if len(result.RemoteIdentity.MemberCredential) == 0 {
		return nil, errors.New("path result remote member credential is required")
	}

	credential, err := enroll.UnmarshalMemberCredential(result.RemoteIdentity.MemberCredential)
	if err != nil {
		return nil, fmt.Errorf("unmarshal remote member credential: %w", err)
	}
	if err := enroll.VerifyMemberCredential(credential, cfg.AuthorityEd25519Pub); err != nil {
		return nil, fmt.Errorf("verify remote member credential: %w", err)
	}
	if credential.NetworkID != networkID {
		return nil, errors.New("remote member credential network id mismatch")
	}
	remotePeerID, err := credential.PeerID()
	if err != nil {
		return nil, fmt.Errorf("derive remote peer id: %w", err)
	}
	if remotePeerID != result.RemoteIdentity.PeerID {
		return nil, errors.New("remote peer id mismatch")
	}
	remotePub := append(ed25519.PublicKey(nil), credential.SubjectEd25519Pub...)

	localKeys, err := cfg.Store.LoadDeviceKeys()
	if err != nil {
		return nil, fmt.Errorf("load local device keys: %w", err)
	}
	localPriv, err := localKeys.Ed25519PrivateKey()
	if err != nil {
		return nil, fmt.Errorf("derive local ed25519 private key: %w", err)
	}
	selfCredentialBytes, err := cfg.Store.LoadSelfMemberCredential(cfg.NetworkID)
	if err != nil {
		return nil, fmt.Errorf("load self member credential: %w", err)
	}
	selfCredential, err := enroll.UnmarshalMemberCredential(selfCredentialBytes)
	if err != nil {
		return nil, fmt.Errorf("unmarshal self member credential: %w", err)
	}
	if err := enroll.VerifyMemberCredential(selfCredential, cfg.AuthorityEd25519Pub); err != nil {
		return nil, fmt.Errorf("verify self member credential: %w", err)
	}
	if selfCredential.NetworkID != networkID {
		return nil, errors.New("self member credential network id mismatch")
	}
	selfPeerID, err := selfCredential.PeerID()
	if err != nil {
		return nil, fmt.Errorf("derive self peer id: %w", err)
	}
	localPeerID, err := localKeys.PeerID()
	if err != nil {
		return nil, fmt.Errorf("derive local peer id: %w", err)
	}
	if selfPeerID != localPeerID {
		return nil, errors.New("local self member credential does not match device keys")
	}
	localCert, err := tlsutil.NewEd25519SelfSignedTLSCertificate(localPriv, "session")
	if err != nil {
		return nil, fmt.Errorf("create local tls certificate: %w", err)
	}
	tlsConfig := &tls.Config{
		MinVersion:   tls.VersionTLS13,
		MaxVersion:   tls.VersionTLS13,
		ClientAuth:   tls.RequireAnyClientCert,
		Certificates: []tls.Certificate{localCert},
		NextProtos:   []string{sessionALPN},
		VerifyPeerCertificate: func(rawCerts [][]byte, verifiedChains [][]*x509.Certificate) error {
			_ = verifiedChains
			peerPub, err := tlsutil.PeerEd25519PublicKey(rawCerts)
			if err != nil {
				return err
			}
			if !bytes.Equal(peerPub, remotePub) {
				return errors.New("pinned peer identity mismatch")
			}
			return nil
		},
	}
	if asClient {
		tlsConfig.InsecureSkipVerify = true
	}

	transport, err := upgradeTransport(ctx, result, tlsConfig, asClient)
	if err != nil {
		return nil, err
	}
	tlsConn, ok := transport.Conn.(*tls.Conn)
	if !ok {
		_ = transport.Close()
		return nil, errors.New("unexpected transport conn type")
	}
	if err := tlsConn.HandshakeContext(ctx); err != nil {
		_ = transport.Close()
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		return nil, fmt.Errorf("tls handshake: %w", err)
	}
	_ = tlsConn.SetDeadline(time.Time{})

	muxCfg := yamux.DefaultConfig()
	muxCfg.LogOutput = io.Discard
	muxCfg.MaxStreamWindowSize = 6 * 1024 * 1024
	var mux *yamux.Session
	if asClient {
		mux, err = yamux.Client(tlsConn, muxCfg)
	} else {
		mux, err = yamux.Server(tlsConn, muxCfg)
	}
	if err != nil {
		_ = transport.Close()
		return nil, fmt.Errorf("create yamux session: %w", err)
	}

	sess := &peerSession{
		key:          dataplane.SessionKey{RemotePeerID: result.RemoteIdentity.PeerID, Protocol: dataplane.ProtocolKCP, SecurityID: result.RemoteIdentity.PeerID, PathFamily: pathFamilyFromRemoteAddr(result.RemoteAddr)}.Normalize(),
		sess:         mux,
		transport:    transport,
		pathFacts:    sessionPathFactsFromResult(result),
		lastActivity: time.Now(),
		done:         make(chan struct{}),
	}
	sess.startIdleCloser(cfg.IdleTimeout)
	return sess, nil
}

func sessionPathFactsFromResult(result punch.PathResult) dataplane.SessionPathFacts {
	var facts dataplane.SessionPathFacts
	if result.Conn != nil && result.Conn.LocalAddr() != nil {
		facts.LocalEndpoint = result.Conn.LocalAddr().String()
	}
	if result.RemoteAddr != nil {
		facts.RemoteEndpoint = result.RemoteAddr.String()
	}
	facts.SelectedPath = result.Evidence.SelectedPath
	return facts.Normalize()
}

func upgradeTransport(ctx context.Context, result punch.PathResult, tlsConfig *tls.Config, asClient bool) (*ownedConn, error) {
	if asClient {
		return dialTransport(ctx, result, tlsConfig)
	}
	return acceptTransport(ctx, result, tlsConfig)
}

func dialTransport(ctx context.Context, result punch.PathResult, tlsConfig *tls.Config) (*ownedConn, error) {
	kcpConn, err := netutil.NewKCPConnFromUDP(result.Conn, false, result.RemoteAddr.String())
	if err != nil {
		return nil, fmt.Errorf("open kcp conn: %w", err)
	}
	applyKCPDefaultsToConn(kcpConn)
	if oob, ok := kcpConn.(interface{ SendOOB([]byte) error }); ok {
		_ = oob.SendOOB([]byte{0})
	}
	if deadline, ok := ctx.Deadline(); ok {
		if err := kcpConn.SetDeadline(deadline); err != nil {
			_ = kcpConn.Close()
			return nil, err
		}
	}
	tlsConn := tls.Client(kcpConn, tlsConfig)
	return &ownedConn{
		Conn:    tlsConn,
		closers: []io.Closer{tlsConn},
	}, nil
}

func acceptTransport(ctx context.Context, result punch.PathResult, tlsConfig *tls.Config) (*ownedConn, error) {
	ln, err := kcp.ServeConn(nil, 10, 3, result.Conn)
	if err != nil {
		return nil, fmt.Errorf("serve kcp conn: %w", err)
	}
	for {
		if err := ctx.Err(); err != nil {
			_ = ln.Close()
			return nil, err
		}

		_ = ln.SetDeadline(time.Now().Add(250 * time.Millisecond))
		kcpSess, err := ln.AcceptKCP()
		if err != nil {
			var netErr net.Error
			if errors.As(err, &netErr) && netErr.Timeout() {
				continue
			}
			_ = ln.Close()
			return nil, err
		}
		if result.RemoteAddr != nil && kcpSess.RemoteAddr().String() != result.RemoteAddr.String() {
			_ = kcpSess.Close()
			continue
		}
		_ = ln.SetDeadline(time.Time{})
		applyKCPDefaults(kcpSess)
		if deadline, ok := ctx.Deadline(); ok {
			if err := kcpSess.SetDeadline(deadline); err != nil {
				_ = kcpSess.Close()
				_ = ln.Close()
				return nil, err
			}
		}
		tlsConn := tls.Server(kcpSess, tlsConfig)
		return &ownedConn{
			Conn:    tlsConn,
			closers: []io.Closer{tlsConn, kcpSess, ln},
		}, nil
	}
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

func applyKCPDefaultsToConn(conn net.Conn) {
	type kcpConfigurer interface {
		SetStreamMode(bool)
		SetWriteDelay(bool)
		SetNoDelay(int, int, int, int)
		SetMtu(int)
		SetWindowSize(int, int)
		SetACKNoDelay(bool)
	}
	if sess, ok := conn.(kcpConfigurer); ok {
		sess.SetStreamMode(true)
		sess.SetWriteDelay(true)
		sess.SetNoDelay(1, 20, 2, 1)
		sess.SetMtu(1350)
		sess.SetWindowSize(1024, 1024)
		sess.SetACKNoDelay(false)
	}
}

func pathFamilyFromRemoteAddr(addr *net.UDPAddr) dataplane.PathFamily {
	if addr == nil {
		return dataplane.PathFamilyUnknown
	}
	if addr.IP != nil && addr.IP.To4() == nil {
		return dataplane.PathFamilyUDP6
	}
	return dataplane.PathFamilyUDP4
}

func normalizeCloseReason(reason dataplane.CloseReason) dataplane.CloseReason {
	if reason == "" {
		return dataplane.CloseReasonTransportFatal
	}
	return reason
}
