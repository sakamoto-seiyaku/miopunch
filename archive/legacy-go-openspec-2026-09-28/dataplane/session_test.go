package dataplane

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"testing"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func TestTLSSession_LogicalStreamCloseKeepsSessionUsable(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)

	clientConn, serverConn := net.Pipe()
	cfg := testSessionConfig(ProtocolTLS, PathFamilyTCP4)

	serverCh := make(chan sessionResult, 1)
	go func() {
		sess, err := ServeTLSSession(ctx, cfg, []connectivity.TCPConn{
			{Conn: serverConn, Origin: connectivity.TCPConnOriginAccept},
		}, nil)
		serverCh <- sessionResult{session: sess, err: err}
	}()

	clientSess, err := DialTLSSession(ctx, cfg, []connectivity.TCPConn{
		{Conn: clientConn, Origin: connectivity.TCPConnOriginDial},
	}, nil)
	if err != nil {
		t.Fatalf("DialTLSSession() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = clientSess.Close(CloseReasonDaemonShutdown) })

	serverRes := <-serverCh
	if serverRes.err != nil {
		t.Fatalf("ServeTLSSession() error = %v, want nil", serverRes.err)
	}
	t.Cleanup(func() { _ = serverRes.session.Close(CloseReasonDaemonShutdown) })

	runShellPingStreams(t, ctx, clientSess, serverRes.session, 2)
}

func TestKCPSession_StreamOpenHelloPingSequential(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)

	clientUDP := listenLocalUDP(t)
	serverUDP := listenLocalUDP(t)

	clientRemote := serverUDP.LocalAddr().(*net.UDPAddr)
	serverRemote := clientUDP.LocalAddr().(*net.UDPAddr)

	cfg := testSessionConfig(ProtocolKCP, PathFamilyUDP4)

	serverCh := make(chan sessionResult, 1)
	go func() {
		sess, err := ServeSession(ctx, cfg, serverUDP, serverRemote, nil)
		serverCh <- sessionResult{session: sess, err: err}
	}()

	clientSess, err := DialSession(ctx, cfg, clientUDP, clientRemote, nil)
	if err != nil {
		t.Fatalf("DialSession(kcp) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = clientSess.Close(CloseReasonDaemonShutdown) })

	serverRes := <-serverCh
	if serverRes.err != nil {
		t.Fatalf("ServeSession(kcp) error = %v, want nil", serverRes.err)
	}
	t.Cleanup(func() { _ = serverRes.session.Close(CloseReasonDaemonShutdown) })

	runShellPingStreams(t, ctx, clientSess, serverRes.session, 2)
}

func TestSessionSummaryIncludesTLSEndpointFacts(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)

	ln, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("Listen(tcp4, 127.0.0.1:0) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	cfg := testSessionConfig(ProtocolTLS, PathFamilyTCP4)
	serverCh := make(chan sessionResult, 1)
	go func() {
		conn, err := ln.Accept()
		if err != nil {
			serverCh <- sessionResult{err: err}
			return
		}
		sess, err := ServeTLSSession(ctx, cfg, []connectivity.TCPConn{
			{Conn: conn, Origin: connectivity.TCPConnOriginAccept},
		}, nil)
		serverCh <- sessionResult{session: sess, err: err}
	}()

	clientConn, err := net.Dial("tcp4", ln.Addr().String())
	if err != nil {
		t.Fatalf("Dial(tcp4, %s) error = %v, want nil", ln.Addr(), err)
	}
	wantLocal := clientConn.LocalAddr().String()
	wantRemote := clientConn.RemoteAddr().String()

	clientSess, err := DialTLSSession(ctx, cfg, []connectivity.TCPConn{
		{Conn: clientConn, Origin: connectivity.TCPConnOriginDial},
	}, nil)
	if err != nil {
		t.Fatalf("DialTLSSession() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = clientSess.Close(CloseReasonDaemonShutdown) })

	serverRes := <-serverCh
	if serverRes.err != nil {
		t.Fatalf("ServeTLSSession() error = %v, want nil", serverRes.err)
	}
	t.Cleanup(func() { _ = serverRes.session.Close(CloseReasonDaemonShutdown) })

	manager := NewSessionManager()
	manager.Put(clientSess)
	summaries := manager.ListAllSummaries()
	if len(summaries) != 1 {
		t.Fatalf("ListAllSummaries() length = %d, want 1: %#v", len(summaries), summaries)
	}
	got := summaries[0].PathFacts.Normalize()
	if got.LocalEndpoint != wantLocal || got.RemoteEndpoint != wantRemote {
		t.Fatalf("ListAllSummaries()[0].PathFacts = %+v, want local %q remote %q", got, wantLocal, wantRemote)
	}
	if got.Port == "" {
		t.Fatalf("ListAllSummaries()[0].PathFacts.Port = empty, want remote endpoint port")
	}
}

func TestSessionSummaryIncludesUDPEndpointFacts(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)

	clientUDP := listenLocalUDP(t)
	serverUDP := listenLocalUDP(t)

	clientRemote := serverUDP.LocalAddr().(*net.UDPAddr)
	serverRemote := clientUDP.LocalAddr().(*net.UDPAddr)

	cfg := testSessionConfig(ProtocolKCP, PathFamilyUDP4)
	serverCh := make(chan sessionResult, 1)
	go func() {
		sess, err := ServeSession(ctx, cfg, serverUDP, serverRemote, nil)
		serverCh <- sessionResult{session: sess, err: err}
	}()

	clientSess, err := DialSession(ctx, cfg, clientUDP, clientRemote, nil)
	if err != nil {
		t.Fatalf("DialSession(kcp) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = clientSess.Close(CloseReasonDaemonShutdown) })

	serverRes := <-serverCh
	if serverRes.err != nil {
		t.Fatalf("ServeSession(kcp) error = %v, want nil", serverRes.err)
	}
	t.Cleanup(func() { _ = serverRes.session.Close(CloseReasonDaemonShutdown) })

	manager := NewSessionManager()
	manager.Put(clientSess)
	summaries := manager.ListAllSummaries()
	if len(summaries) != 1 {
		t.Fatalf("ListAllSummaries() length = %d, want 1: %#v", len(summaries), summaries)
	}
	got := summaries[0].PathFacts.Normalize()
	if got.LocalEndpoint != clientUDP.LocalAddr().String() || got.RemoteEndpoint != clientRemote.String() {
		t.Fatalf("ListAllSummaries()[0].PathFacts = %+v, want local %q remote %q", got, clientUDP.LocalAddr(), clientRemote)
	}
	if got.Port == "" {
		t.Fatalf("ListAllSummaries()[0].PathFacts.Port = empty, want remote endpoint port")
	}
}

func TestTLSSession_LogicalStreamActivityKeepsSessionHealthy(t *testing.T) {
	t.Parallel()

	const idleTimeout = 1200 * time.Millisecond

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)

	clientSess, serverSess := mustNewTLSPeerSessions(t, ctx, idleTimeout)
	clientStream, serverStream := openShellStreamPair(t, ctx, clientSess, serverSess, "activity")
	defer func() { _ = clientStream.Close() }()
	defer func() { _ = serverStream.Close() }()

	serverErrCh := make(chan error, 1)
	go func() {
		serverErrCh <- serveShellPingLoop(ctx, serverStream, 5)
	}()

	start := time.Now()
	for i := 0; i < 5; i++ {
		if err := shellproto.WriteJSON(clientStream, shellproto.Control{Op: shellproto.OpPing}); err != nil {
			t.Fatalf("WriteJSON(ping, seq=%d) error = %v, want nil", i, err)
		}
		kind, payload, err := shellproto.ReadFrame(clientStream)
		if err != nil {
			t.Fatalf("ReadFrame(ping response, seq=%d) error = %v, want nil", i, err)
		}
		if kind != shellproto.KindJSON {
			t.Fatalf("ReadFrame(ping response, seq=%d) kind = %d, want %d", i, kind, shellproto.KindJSON)
		}
		var resp shellproto.Control
		if err := json.Unmarshal(payload, &resp); err != nil {
			t.Fatalf("Unmarshal(ping response, seq=%d) error = %v, want nil", i, err)
		}
		if resp.Op != shellproto.OpPing || !resp.OK {
			t.Fatalf("Unmarshal(ping response, seq=%d) = %+v, want ping ok", i, resp)
		}
		if i < 4 {
			time.Sleep(550 * time.Millisecond)
		}
	}

	if elapsed := time.Since(start); elapsed <= idleTimeout {
		t.Fatalf("activity loop elapsed = %v, want > %v to cross idle timeout window", elapsed, idleTimeout)
	}
	if !clientSess.Healthy() {
		t.Fatalf("client session unhealthy after sustained stream activity, want healthy")
	}
	if !serverSess.Healthy() {
		t.Fatalf("server session unhealthy after sustained stream activity, want healthy")
	}
	if err := <-serverErrCh; err != nil {
		t.Fatalf("serveShellPingLoop() error = %v, want nil", err)
	}
}

func TestTLSSession_IdleTimeoutClosesTrulyInactiveSession(t *testing.T) {
	t.Parallel()

	const idleTimeout = 1200 * time.Millisecond

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)

	clientSess, serverSess := mustNewTLSPeerSessions(t, ctx, idleTimeout)

	waitForSessionCloseReason(t, clientSess, CloseReasonIdleTimeout, 4*time.Second)
	waitForSessionCloseReason(t, serverSess, CloseReasonIdleTimeout, 4*time.Second)
}

type sessionResult struct {
	session PeerSession
	err     error
}

func testSessionConfig(proto Protocol, family PathFamily) Config {
	return Config{
		Proto:        proto,
		RemotePeerID: "peer-a",
		SecurityID:   "sid-1",
		SecretKey:    []byte("secret"),
		PathFamily:   family,
		IdleTimeout:  time.Minute,
	}
}

func testSessionConfigWithIdleTimeout(proto Protocol, family PathFamily, idleTimeout time.Duration) Config {
	cfg := testSessionConfig(proto, family)
	cfg.IdleTimeout = idleTimeout
	return cfg
}

func mustNewTLSPeerSessions(t *testing.T, ctx context.Context, idleTimeout time.Duration) (PeerSession, PeerSession) {
	t.Helper()

	clientConn, serverConn := net.Pipe()
	cfg := testSessionConfigWithIdleTimeout(ProtocolTLS, PathFamilyTCP4, idleTimeout)

	serverCh := make(chan sessionResult, 1)
	go func() {
		sess, err := ServeTLSSession(ctx, cfg, []connectivity.TCPConn{
			{Conn: serverConn, Origin: connectivity.TCPConnOriginAccept},
		}, nil)
		serverCh <- sessionResult{session: sess, err: err}
	}()

	clientSess, err := DialTLSSession(ctx, cfg, []connectivity.TCPConn{
		{Conn: clientConn, Origin: connectivity.TCPConnOriginDial},
	}, nil)
	if err != nil {
		t.Fatalf("DialTLSSession() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = clientSess.Close(CloseReasonDaemonShutdown) })

	serverRes := <-serverCh
	if serverRes.err != nil {
		t.Fatalf("ServeTLSSession() error = %v, want nil", serverRes.err)
	}
	t.Cleanup(func() { _ = serverRes.session.Close(CloseReasonDaemonShutdown) })

	return clientSess, serverRes.session
}

func listenLocalUDP(t *testing.T) *net.UDPConn {
	t.Helper()

	addr, err := net.ResolveUDPAddr("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("ResolveUDPAddr(127.0.0.1:0) error = %v, want nil", err)
	}
	conn, err := net.ListenUDP("udp", addr)
	if err != nil {
		t.Fatalf("ListenUDP(127.0.0.1:0) error = %v, want nil", err)
	}
	return conn
}

func openShellStreamPair(t *testing.T, ctx context.Context, clientSess PeerSession, serverSess PeerSession, seq string) (io.ReadWriteCloser, io.ReadWriteCloser) {
	t.Helper()

	serverAcceptedCh := make(chan *AcceptedStream, 1)
	serverErrCh := make(chan error, 1)
	go func() {
		accepted, err := serverSess.AcceptStream(ctx)
		if err != nil {
			serverErrCh <- err
			return
		}
		serverAcceptedCh <- accepted
	}()

	clientStream, err := clientSess.OpenStream(ctx, StreamOpen{
		Kind: StreamKindShellV0,
		Metadata: map[string]string{
			"peer_id": "peer-a",
			"op":      shellproto.OpPing,
			"seq":     seq,
		},
	})
	if err != nil {
		t.Fatalf("PeerSession.OpenStream(seq=%s) error = %v, want nil", seq, err)
	}

	var hello shellproto.Control
	serverStream := waitAcceptedShellStream(t, serverAcceptedCh, serverErrCh)
	if err := shellproto.WriteJSON(serverStream, shellproto.Control{Op: shellproto.OpHello, OK: true}); err != nil {
		_ = clientStream.Close()
		_ = serverStream.Close()
		t.Fatalf("WriteJSON(hello, seq=%s) error = %v, want nil", seq, err)
	}
	if err := shellproto.ReadJSON(clientStream, &hello); err != nil {
		_ = clientStream.Close()
		_ = serverStream.Close()
		t.Fatalf("ReadJSON(hello, seq=%s) error = %v, want nil", seq, err)
	}
	if hello.Op != shellproto.OpHello || !hello.OK {
		_ = clientStream.Close()
		_ = serverStream.Close()
		t.Fatalf("ReadJSON(hello, seq=%s) = %+v, want hello ok", seq, hello)
	}

	return clientStream, serverStream
}

func waitAcceptedShellStream(t *testing.T, acceptedCh <-chan *AcceptedStream, errCh <-chan error) io.ReadWriteCloser {
	t.Helper()

	select {
	case accepted := <-acceptedCh:
		if accepted == nil {
			t.Fatalf("accepted stream = nil, want non-nil")
		}
		if accepted.Open.Kind != StreamKindShellV0 {
			t.Fatalf("accepted stream kind = %q, want %q", accepted.Open.Kind, StreamKindShellV0)
		}
		return accepted.Stream
	case err := <-errCh:
		t.Fatalf("AcceptStream() error = %v, want nil", err)
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for accepted stream")
	}

	return nil
}

func runShellPingStreams(t *testing.T, ctx context.Context, clientSess PeerSession, serverSess PeerSession, count int) {
	t.Helper()

	serverErrCh := make(chan error, 1)
	go func() {
		serverErrCh <- serveShellPingStreams(ctx, serverSess, count)
	}()

	for i := 0; i < count; i++ {
		stream, err := clientSess.OpenStream(ctx, StreamOpen{
			Kind: StreamKindShellV0,
			Metadata: map[string]string{
				"peer_id": "peer-a",
				"op":      shellproto.OpPing,
				"seq":     fmt.Sprintf("%d", i),
			},
		})
		if err != nil {
			t.Fatalf("PeerSession.OpenStream(seq=%d) error = %v, want nil", i, err)
		}

		var hello shellproto.Control
		if err := shellproto.ReadJSON(stream, &hello); err != nil {
			t.Fatalf("ReadJSON(hello, seq=%d) error = %v, want nil", i, err)
		}
		if hello.Op != shellproto.OpHello || !hello.OK {
			t.Fatalf("ReadJSON(hello, seq=%d) = %+v, want hello ok", i, hello)
		}
		if err := shellproto.WriteJSON(stream, shellproto.Control{Op: shellproto.OpPing}); err != nil {
			t.Fatalf("WriteJSON(ping, seq=%d) error = %v, want nil", i, err)
		}

		kind, payload, err := shellproto.ReadFrame(stream)
		if err != nil {
			t.Fatalf("ReadFrame(ping response, seq=%d) error = %v, want nil", i, err)
		}
		if kind != shellproto.KindJSON {
			t.Fatalf("ReadFrame(ping response, seq=%d) kind = %d, want %d", i, kind, shellproto.KindJSON)
		}
		var resp shellproto.Control
		if err := json.Unmarshal(payload, &resp); err != nil {
			t.Fatalf("Unmarshal(ping response, seq=%d) error = %v, want nil", i, err)
		}
		if resp.Op != shellproto.OpPing || !resp.OK {
			t.Fatalf("Unmarshal(ping response, seq=%d) = %+v, want ping ok", i, resp)
		}
		if err := stream.Close(); err != nil {
			t.Fatalf("stream.Close(seq=%d) error = %v, want nil", i, err)
		}
		if !clientSess.Healthy() {
			t.Fatalf("PeerSession.Healthy(seq=%d) = false, want true after stream close", i)
		}
	}

	if err := <-serverErrCh; err != nil {
		t.Fatalf("serveShellPingStreams() error = %v, want nil", err)
	}
}

func serveShellPingStreams(ctx context.Context, sess PeerSession, count int) error {
	for i := 0; i < count; i++ {
		accepted, err := sess.AcceptStream(ctx)
		if err != nil {
			return fmt.Errorf("accept stream %d: %w", i, err)
		}
		if accepted.Open.Kind != StreamKindShellV0 {
			return fmt.Errorf("accepted stream %d kind = %q, want %q", i, accepted.Open.Kind, StreamKindShellV0)
		}
		if accepted.Open.Metadata["op"] != shellproto.OpPing {
			return fmt.Errorf("accepted stream %d op = %q, want %q", i, accepted.Open.Metadata["op"], shellproto.OpPing)
		}
		if err := shellproto.WriteJSON(accepted.Stream, shellproto.Control{Op: shellproto.OpHello, OK: true}); err != nil {
			_ = accepted.Stream.Close()
			return fmt.Errorf("write hello %d: %w", i, err)
		}

		var req shellproto.Control
		if err := shellproto.ReadJSON(accepted.Stream, &req); err != nil {
			_ = accepted.Stream.Close()
			return fmt.Errorf("read ping %d: %w", i, err)
		}
		if req.Op != shellproto.OpPing {
			_ = accepted.Stream.Close()
			return fmt.Errorf("read ping %d op = %q, want %q", i, req.Op, shellproto.OpPing)
		}
		if err := shellproto.WriteJSON(accepted.Stream, shellproto.Control{Op: shellproto.OpPing, OK: true}); err != nil {
			_ = accepted.Stream.Close()
			return fmt.Errorf("write ping response %d: %w", i, err)
		}
		if err := accepted.Stream.Close(); err != nil {
			return fmt.Errorf("close accepted stream %d: %w", i, err)
		}
		if !sess.Healthy() {
			return fmt.Errorf("session unhealthy after accepted stream %d close", i)
		}
	}
	return nil
}

func serveShellPingLoop(ctx context.Context, stream io.ReadWriteCloser, count int) error {
	for i := 0; i < count; i++ {
		var req shellproto.Control
		if err := shellproto.ReadJSON(stream, &req); err != nil {
			return fmt.Errorf("read ping %d: %w", i, err)
		}
		if req.Op != shellproto.OpPing {
			return fmt.Errorf("read ping %d op = %q, want %q", i, req.Op, shellproto.OpPing)
		}
		if err := shellproto.WriteJSON(stream, shellproto.Control{Op: shellproto.OpPing, OK: true}); err != nil {
			return fmt.Errorf("write ping response %d: %w", i, err)
		}
	}
	return nil
}

func waitForSessionCloseReason(t *testing.T, sess PeerSession, want CloseReason, timeout time.Duration) {
	t.Helper()

	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		if !sess.Healthy() {
			if got := sess.CloseReason(); got != want {
				t.Fatalf("PeerSession.CloseReason() = %q, want %q", got, want)
			}
			return
		}
		time.Sleep(25 * time.Millisecond)
	}
	t.Fatalf("session remained healthy after %v, want close reason %q", timeout, want)
}

type failingReadWriteCloser struct {
	closeErr error
}

func (f failingReadWriteCloser) Read(p []byte) (int, error)  { return 0, io.EOF }
func (f failingReadWriteCloser) Write(p []byte) (int, error) { return len(p), nil }
func (f failingReadWriteCloser) Close() error                { return f.closeErr }

func TestLogicalStreamClose_EventIsInfoAndCarriesCloseErr(t *testing.T) {
	t.Parallel()

	var buf bytes.Buffer
	em := event.NewEmitter(&buf, "test")

	closeErr := errors.New("close failed")
	ls := &logicalStream{
		rwc: failingReadWriteCloser{closeErr: closeErr},
		em:  em,
		key: SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     ProtocolTLS,
			SecurityID:   "sid-1",
			PathFamily:   PathFamilyTCP4,
		},
		open: StreamOpen{Kind: StreamKindShellV0},
	}
	if err := ls.Close(); err == nil {
		t.Fatalf("logicalStream.Close() = nil, want error")
	}

	var ev event.Event
	dec := json.NewDecoder(&buf)
	if err := dec.Decode(&ev); err != nil {
		t.Fatalf("decode event json error = %v, want nil", err)
	}
	if ev.Kind != event.KindInfo {
		t.Fatalf("event.Kind = %q, want %q", ev.Kind, event.KindInfo)
	}
	if ev.Err != "" {
		t.Fatalf("event.Err = %q, want empty", ev.Err)
	}
	if ev.Name != "transport.stream_close" {
		t.Fatalf("event.Name = %q, want %q", ev.Name, "transport.stream_close")
	}
	got, ok := ev.KVs["close_err"]
	if !ok {
		t.Fatalf("event.KVs.close_err missing, want present")
	}
	if got != closeErr.Error() {
		t.Fatalf("event.KVs.close_err = %v, want %q", got, closeErr.Error())
	}
}
