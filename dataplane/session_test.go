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
