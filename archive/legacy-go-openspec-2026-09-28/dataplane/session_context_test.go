package dataplane

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"testing"
	"time"
)

func TestWriteStreamOpenWithContext_ContextCanceled(t *testing.T) {
	t.Parallel()

	local, remote := net.Pipe()
	t.Cleanup(func() { _ = local.Close() })
	t.Cleanup(func() { _ = remote.Close() })

	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	time.AfterFunc(75*time.Millisecond, cancel)

	errCh := make(chan error, 1)
	go func() {
		errCh <- writeStreamOpenWithContext(ctx, local, StreamOpen{Kind: StreamKindShellV0})
	}()

	select {
	case err := <-errCh:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("writeStreamOpenWithContext() error = %v, want %v", err, context.Canceled)
		}
	case <-time.After(2 * time.Second):
		t.Fatalf("writeStreamOpenWithContext() timed out waiting for context cancellation")
	}
}

func TestReadStreamOpenWithContext_ContextDeadlineExceeded(t *testing.T) {
	t.Parallel()

	local, remote := net.Pipe()
	t.Cleanup(func() { _ = local.Close() })
	t.Cleanup(func() { _ = remote.Close() })

	ctx, cancel := context.WithTimeout(context.Background(), 75*time.Millisecond)
	t.Cleanup(cancel)

	errCh := make(chan error, 1)
	go func() {
		_, err := readStreamOpenWithContext(ctx, local)
		errCh <- err
	}()

	select {
	case err := <-errCh:
		if !errors.Is(err, context.DeadlineExceeded) {
			t.Fatalf("readStreamOpenWithContext() error = %v, want %v", err, context.DeadlineExceeded)
		}
	case <-time.After(2 * time.Second):
		t.Fatalf("readStreamOpenWithContext() timed out waiting for context deadline")
	}
}

func TestWriteStreamOpenWithContext_ClearsWriteDeadlineAfterSuccess(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	t.Cleanup(cancel)

	stream := &recordingWriteStream{}
	err := writeStreamOpenWithContext(ctx, stream, StreamOpen{
		Kind:     StreamKindShellV0,
		Metadata: map[string]string{"op": "ping"},
	})
	if err != nil {
		t.Fatalf("writeStreamOpenWithContext() error = %v, want nil", err)
	}
	if got := len(stream.deadlines); got < 2 {
		t.Fatalf("writeStreamOpenWithContext() deadline call count = %d, want at least 2", got)
	}
	if got := stream.deadlines[0]; got.IsZero() {
		t.Fatalf("writeStreamOpenWithContext() first deadline = zero, want non-zero")
	}
	if got := stream.deadlines[len(stream.deadlines)-1]; !got.IsZero() {
		t.Fatalf("writeStreamOpenWithContext() last deadline = %v, want zero", got)
	}
	if got := stream.Len(); got == 0 {
		t.Fatalf("writeStreamOpenWithContext() wrote %d bytes, want > 0", got)
	}
}

func TestReadStreamOpenWithContext_ClearsReadDeadlineAfterSuccess(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	t.Cleanup(cancel)

	want := StreamOpen{
		Kind:     StreamKindShellV0,
		Metadata: map[string]string{"op": "ping", "trace": "trace-01"},
	}
	frame, err := marshalStreamOpen(want)
	if err != nil {
		t.Fatalf("marshalStreamOpen() error = %v, want nil", err)
	}
	var buf bytes.Buffer
	if err := writeFrame(&buf, frame); err != nil {
		t.Fatalf("writeFrame() error = %v, want nil", err)
	}

	stream := &recordingReadStream{reader: bytes.NewReader(buf.Bytes())}
	got, err := readStreamOpenWithContext(ctx, stream)
	if err != nil {
		t.Fatalf("readStreamOpenWithContext() error = %v, want nil", err)
	}
	if got.Kind != want.Kind {
		t.Fatalf("readStreamOpenWithContext() kind = %q, want %q", got.Kind, want.Kind)
	}
	if got.Metadata["trace"] != want.Metadata["trace"] {
		t.Fatalf("readStreamOpenWithContext() trace = %q, want %q", got.Metadata["trace"], want.Metadata["trace"])
	}
	if got := len(stream.deadlines); got < 2 {
		t.Fatalf("readStreamOpenWithContext() deadline call count = %d, want at least 2", got)
	}
	if got := stream.deadlines[0]; got.IsZero() {
		t.Fatalf("readStreamOpenWithContext() first deadline = zero, want non-zero")
	}
	if got := stream.deadlines[len(stream.deadlines)-1]; !got.IsZero() {
		t.Fatalf("readStreamOpenWithContext() last deadline = %v, want zero", got)
	}
}

func TestWriteStreamOpenWithContext_PreservesNonContextError(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	t.Cleanup(cancel)

	wantErr := errors.New("write failed")
	stream := &recordingWriteStream{writeErr: wantErr}
	err := writeStreamOpenWithContext(ctx, stream, StreamOpen{Kind: StreamKindShellV0})
	if !errors.Is(err, wantErr) {
		t.Fatalf("writeStreamOpenWithContext() error = %v, want %v", err, wantErr)
	}
}

func TestKCPSession_AcceptStreamContextCancellationKeepsSessionUsable(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	t.Cleanup(cancel)

	clientSess, serverSess := mustNewKCPPeerSessions(t, ctx)
	rawStream, err := clientSess.sess.OpenStream()
	if err != nil {
		t.Fatalf("client yamux OpenStream() error = %v, want nil", err)
	}

	shortCtx, shortCancel := context.WithTimeout(ctx, 150*time.Millisecond)
	t.Cleanup(shortCancel)

	_, err = serverSess.AcceptStream(shortCtx)
	if !errors.Is(err, context.DeadlineExceeded) {
		_ = rawStream.Close()
		t.Fatalf("server AcceptStream() error = %v, want %v", err, context.DeadlineExceeded)
	}
	if err := rawStream.Close(); err != nil {
		t.Fatalf("rawStream.Close() error = %v, want nil", err)
	}
	if !clientSess.Healthy() {
		t.Fatalf("clientSess.Healthy() = false, want true after context cancellation")
	}
	if !serverSess.Healthy() {
		t.Fatalf("serverSess.Healthy() = false, want true after context cancellation")
	}

	runShellPingStreams(t, ctx, clientSess, serverSess, 1)
}

func TestQUICSession_AcceptStreamContextCancellationReturnsContextError(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	t.Cleanup(cancel)

	clientSess, serverSess := mustNewQUICPeerSessions(t, ctx)
	rawStream, err := clientSess.conn.OpenStreamSync(ctx)
	if err != nil {
		t.Fatalf("client quic OpenStreamSync() error = %v, want nil", err)
	}

	shortCtx, shortCancel := context.WithTimeout(ctx, 150*time.Millisecond)
	t.Cleanup(shortCancel)

	_, err = serverSess.AcceptStream(shortCtx)
	if !errors.Is(err, context.DeadlineExceeded) {
		_ = rawStream.Close()
		t.Fatalf("server AcceptStream() error = %v, want %v", err, context.DeadlineExceeded)
	}
	if err := rawStream.Close(); err != nil {
		t.Fatalf("rawStream.Close() error = %v, want nil", err)
	}
	if !clientSess.Healthy() {
		t.Fatalf("clientSess.Healthy() = false, want true after context cancellation")
	}
	if !serverSess.Healthy() {
		t.Fatalf("serverSess.Healthy() = false, want true after context cancellation")
	}
}

type recordingWriteStream struct {
	bytes.Buffer
	writeErr  error
	deadlines []time.Time
}

func (s *recordingWriteStream) SetWriteDeadline(t time.Time) error {
	s.deadlines = append(s.deadlines, t)
	return nil
}

func (s *recordingWriteStream) Write(p []byte) (int, error) {
	if s.writeErr != nil {
		return 0, s.writeErr
	}
	return s.Buffer.Write(p)
}

type recordingReadStream struct {
	reader    io.Reader
	deadlines []time.Time
}

func (s *recordingReadStream) SetReadDeadline(t time.Time) error {
	s.deadlines = append(s.deadlines, t)
	return nil
}

func (s *recordingReadStream) Read(p []byte) (int, error) {
	return s.reader.Read(p)
}

func mustNewKCPPeerSessions(t *testing.T, ctx context.Context) (*yamuxPeerSession, *yamuxPeerSession) {
	t.Helper()

	clientUDP := listenLocalUDP(t)
	serverUDP := listenLocalUDP(t)
	t.Cleanup(func() { _ = clientUDP.Close() })
	t.Cleanup(func() { _ = serverUDP.Close() })

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
	clientPeer, ok := clientSess.(*yamuxPeerSession)
	if !ok {
		t.Fatalf("DialSession(kcp) type = %T, want *yamuxPeerSession", clientSess)
	}
	t.Cleanup(func() { _ = clientPeer.Close(CloseReasonDaemonShutdown) })

	serverRes := <-serverCh
	if serverRes.err != nil {
		t.Fatalf("ServeSession(kcp) error = %v, want nil", serverRes.err)
	}
	serverPeer, ok := serverRes.session.(*yamuxPeerSession)
	if !ok {
		t.Fatalf("ServeSession(kcp) type = %T, want *yamuxPeerSession", serverRes.session)
	}
	t.Cleanup(func() { _ = serverPeer.Close(CloseReasonDaemonShutdown) })

	return clientPeer, serverPeer
}

func mustNewQUICPeerSessions(t *testing.T, ctx context.Context) (*quicPeerSession, *quicPeerSession) {
	t.Helper()

	clientUDP := listenLocalUDP(t)
	serverUDP := listenLocalUDP(t)
	t.Cleanup(func() { _ = clientUDP.Close() })
	t.Cleanup(func() { _ = serverUDP.Close() })

	cfg := testSessionConfig(ProtocolQUIC, PathFamilyUDP4)
	serverCh := make(chan sessionResult, 1)
	go func() {
		sess, err := ServeSession(ctx, cfg, serverUDP, nil, nil)
		serverCh <- sessionResult{session: sess, err: err}
	}()

	clientSess, err := DialSession(ctx, cfg, clientUDP, serverUDP.LocalAddr().(*net.UDPAddr), nil)
	if err != nil {
		t.Fatalf("DialSession(quic) error = %v, want nil", err)
	}
	clientPeer, ok := clientSess.(*quicPeerSession)
	if !ok {
		t.Fatalf("DialSession(quic) type = %T, want *quicPeerSession", clientSess)
	}
	t.Cleanup(func() { _ = clientPeer.Close(CloseReasonDaemonShutdown) })

	serverRes := <-serverCh
	if serverRes.err != nil {
		t.Fatalf("ServeSession(quic) error = %v, want nil", serverRes.err)
	}
	serverPeer, ok := serverRes.session.(*quicPeerSession)
	if !ok {
		t.Fatalf("ServeSession(quic) type = %T, want *quicPeerSession", serverRes.session)
	}
	t.Cleanup(func() { _ = serverPeer.Close(CloseReasonDaemonShutdown) })

	return clientPeer, serverPeer
}
