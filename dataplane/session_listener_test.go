package dataplane

import (
	"context"
	"net"
	"testing"
	"time"

	"github.com/apernet/quic-go"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/punching"
	"github.com/miopunch/miopunch/internal/udpowner"
	"github.com/miopunch/miopunch/internal/wire"
)

func TestPeerSessionListener_QUIC_AcceptTwiceAndServeStreams(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	t.Cleanup(cancel)

	serverUDP := listenLocalUDP(t)
	cfg := testSessionConfig(ProtocolQUIC, PathFamilyUDP4)

	ln, err := ListenSessions(ctx, cfg, serverUDP, nil)
	if err != nil {
		t.Fatalf("ListenSessions(quic) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	serverAddr := serverUDP.LocalAddr().(*net.UDPAddr)

	for i := 0; i < 2; i++ {
		acceptCh := make(chan sessionResult, 1)
		go func() {
			sess, err := ln.Accept(ctx)
			acceptCh <- sessionResult{session: sess, err: err}
		}()

		clientUDP := listenLocalUDP(t)
		clientSess, err := DialSession(ctx, cfg, clientUDP, serverAddr, nil)
		if err != nil {
			t.Fatalf("DialSession(quic) error = %v, want nil", err)
		}
		t.Cleanup(func() { _ = clientSess.Close(CloseReasonDaemonShutdown) })

		res := <-acceptCh
		if res.err != nil {
			_ = clientSess.Close(CloseReasonDaemonShutdown)
			t.Fatalf("listener.Accept(quic) error = %v, want nil", res.err)
		}
		t.Cleanup(func() { _ = res.session.Close(CloseReasonDaemonShutdown) })

		runShellPingStreams(t, ctx, clientSess, res.session, 1)

		_ = clientSess.Close(CloseReasonDaemonShutdown)
		_ = res.session.Close(CloseReasonDaemonShutdown)
	}
}

func TestPeerSessionListener_QUIC_TransportOwned_Brutal(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	t.Cleanup(cancel)

	serverUDP := listenLocalUDP(t)
	cfg := testSessionConfig(ProtocolQUIC, PathFamilyUDP4)
	cfg.QuicCC = QUICCCBrutal
	cfg.Brutal.UpBps = 1_000_000
	cfg.Brutal.DownBps = 1_000_000

	serverTr := &quic.Transport{Conn: serverUDP}
	ln, err := ListenSessionsWithQUICTransport(ctx, cfg, serverTr, serverUDP, nil)
	if err != nil {
		t.Fatalf("ListenSessionsWithQUICTransport(quic) error = %v, want nil", err)
	}
	t.Cleanup(func() {
		_ = ln.Close()
		_ = serverTr.Close()
	})

	serverAddr := serverUDP.LocalAddr().(*net.UDPAddr)

	acceptCh := make(chan sessionResult, 1)
	go func() {
		sess, err := ln.Accept(ctx)
		acceptCh <- sessionResult{session: sess, err: err}
	}()

	clientUDP := listenLocalUDP(t)
	clientTr := &quic.Transport{Conn: clientUDP}
	clientSess, err := DialSessionWithQUICTransport(ctx, cfg, clientTr, serverAddr, nil)
	if err != nil {
		_ = clientTr.Close()
		_ = clientUDP.Close()
		t.Fatalf("DialSessionWithQUICTransport(quic brutal) error = %v, want nil", err)
	}
	t.Cleanup(func() {
		_ = clientSess.Close(CloseReasonDaemonShutdown)
		_ = clientTr.Close()
		_ = clientUDP.Close()
	})

	res := <-acceptCh
	if res.err != nil {
		t.Fatalf("listener.Accept(quic brutal) error = %v, want nil", res.err)
	}
	t.Cleanup(func() { _ = res.session.Close(CloseReasonDaemonShutdown) })

	runShellPingStreams(t, ctx, clientSess, res.session, 1)
}

func TestPeerSessionListener_KCP_AcceptTwiceAndServeStreams(t *testing.T) {
	// This test is timing-sensitive (KCP + TLS + yamux). Keep it non-parallel to
	// reduce flakiness under load.
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	t.Cleanup(cancel)

	serverUDP := listenLocalUDP(t)
	cfg := testSessionConfig(ProtocolKCP, PathFamilyUDP4)

	owner, err := udpowner.NewKCPOwner(serverUDP, udpowner.KCPOwnerConfig{
		Traversal: udpowner.DemuxConfig{Key: cfg.SecretKey},
	})
	if err != nil {
		t.Fatalf("NewKCPOwner error = %v, want nil", err)
	}

	ln, err := ListenSessionsWithKCPPacketConn(ctx, cfg, owner.PacketConn(), nil)
	if err != nil {
		t.Fatalf("ListenSessions(kcp) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	serverAddr := serverUDP.LocalAddr().(*net.UDPAddr)

	// Regression: punching packets must not reach the KCP listener accept path.
	{
		punchConn := listenLocalUDP(t)
		defer punchConn.Close()

		msg := &wire.NatHoleSid{TransactionID: "tx", Sid: "sid-1", Response: false, Nonce: "0"}
		data, err := punching.EncodeMessage(msg, cfg.SecretKey)
		if err != nil {
			t.Fatalf("EncodeMessage error = %v, want nil", err)
		}
		if _, err := punchConn.WriteToUDP(data, serverAddr); err != nil {
			t.Fatalf("WriteToUDP(punch) error = %v, want nil", err)
		}

		punchCtx, cancelPunch := context.WithTimeout(ctx, 300*time.Millisecond)
		defer cancelPunch()
		_, err = ln.Accept(punchCtx)
		if err == nil {
			t.Fatalf("unexpected Accept success after only punching packet")
		}
	}

	for i := 0; i < 2; i++ {
		clientUDP := listenLocalUDP(t)
		attemptCtx, cancelAttempt := context.WithTimeout(ctx, 15*time.Second)

		acceptCh := make(chan sessionResult, 1)
		dialCh := make(chan sessionResult, 1)

		go func() {
			sess, err := ln.Accept(attemptCtx)
			acceptCh <- sessionResult{session: sess, err: err}
		}()
		go func() {
			sess, err := DialSession(attemptCtx, cfg, clientUDP, serverAddr, nil)
			dialCh <- sessionResult{session: sess, err: err}
		}()

		var (
			serverRes sessionResult
			clientRes sessionResult
			gotServer bool
			gotClient bool
		)
		for !(gotServer && gotClient) {
			select {
			case serverRes = <-acceptCh:
				gotServer = true
			case clientRes = <-dialCh:
				gotClient = true
			case <-attemptCtx.Done():
				cancelAttempt()
				t.Fatalf(
					"kcp attempt timed out: gotDial=%v gotAccept=%v dial_err=%v accept_err=%v owner=%+v",
					gotClient, gotServer, clientRes.err, serverRes.err, owner.Stats(),
				)
			}
		}
		cancelAttempt()
		if clientRes.err != nil || serverRes.err != nil {
			if clientRes.session != nil {
				_ = clientRes.session.Close(CloseReasonDaemonShutdown)
			} else {
				_ = clientUDP.Close()
			}
			if serverRes.session != nil {
				_ = serverRes.session.Close(CloseReasonDaemonShutdown)
			}
			t.Fatalf("kcp attempt failed: dial_err=%v accept_err=%v", clientRes.err, serverRes.err)
		}

		clientSess := clientRes.session
		serverSess := serverRes.session

		t.Cleanup(func() { _ = clientSess.Close(CloseReasonDaemonShutdown) })
		t.Cleanup(func() { _ = serverSess.Close(CloseReasonDaemonShutdown) })

		runShellPingStreams(t, ctx, clientSess, serverSess, 1)

		_ = clientSess.Close(CloseReasonDaemonShutdown)
		_ = serverSess.Close(CloseReasonDaemonShutdown)
	}
}

func TestPeerSession_KCP_PacketConnDeadlineClearedAfterHandshake(t *testing.T) {
	// This test protects against leaking the handshake context deadline into the
	// long-lived packetconn used by kcp-go.
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	t.Cleanup(cancel)

	serverUDP := listenLocalUDP(t)
	cfg := testSessionConfig(ProtocolKCP, PathFamilyUDP4)

	serverOwner, err := udpowner.NewKCPOwner(serverUDP, udpowner.KCPOwnerConfig{
		Traversal: udpowner.DemuxConfig{Key: cfg.SecretKey},
	})
	if err != nil {
		t.Fatalf("NewKCPOwner(server) error = %v, want nil", err)
	}

	ln, err := ListenSessionsWithKCPPacketConn(ctx, cfg, serverOwner.PacketConn(), nil)
	if err != nil {
		t.Fatalf("ListenSessionsWithKCPPacketConn error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	serverAddr := serverUDP.LocalAddr().(*net.UDPAddr)

	acceptCh := make(chan sessionResult, 1)
	go func() {
		sess, err := ln.Accept(ctx)
		acceptCh <- sessionResult{session: sess, err: err}
	}()

	clientUDP := listenLocalUDP(t)
	clientOwner, err := udpowner.NewKCPOwner(clientUDP, udpowner.KCPOwnerConfig{
		Traversal: udpowner.DemuxConfig{Key: cfg.SecretKey},
	})
	if err != nil {
		t.Fatalf("NewKCPOwner(client) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = clientOwner.Close() })

	handshakeCtx, cancelHandshake := context.WithTimeout(ctx, 1*time.Second)
	t.Cleanup(cancelHandshake)
	handshakeDeadline, _ := handshakeCtx.Deadline()

	clientSess, err := DialSessionWithKCPPacketConn(handshakeCtx, cfg, clientOwner.PacketConn(), serverAddr, nil)
	if err != nil {
		t.Fatalf("DialSessionWithKCPPacketConn error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = clientSess.Close(CloseReasonDaemonShutdown) })

	res := <-acceptCh
	if res.err != nil {
		t.Fatalf("listener.Accept(kcp) error = %v, want nil", res.err)
	}
	serverSess := res.session
	t.Cleanup(func() { _ = serverSess.Close(CloseReasonDaemonShutdown) })

	// Wait until the handshake deadline elapses. If DialSessionWithKCPPacketConn
	// leaked the deadline to the underlying packetconn, the session would start
	// failing reads and become unusable here.
	time.Sleep(time.Until(handshakeDeadline.Add(150 * time.Millisecond)))

	runCtx, cancelRun := context.WithTimeout(ctx, 5*time.Second)
	t.Cleanup(cancelRun)

	runShellPingStreams(t, runCtx, clientSess, serverSess, 1)
}

func TestPeerSessionListener_TLS_AcceptTwiceAndServeStreams(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	t.Cleanup(cancel)

	addr, err := net.ResolveTCPAddr("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("ResolveTCPAddr error = %v, want nil", err)
	}
	tcpLn, err := net.ListenTCP("tcp", addr)
	if err != nil {
		t.Fatalf("ListenTCP error = %v, want nil", err)
	}

	cfg := testSessionConfig(ProtocolTLS, PathFamilyTCP4)
	ln, err := ListenTLSSessions(ctx, cfg, tcpLn, nil)
	if err != nil {
		t.Fatalf("ListenTLSSessions error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	for i := 0; i < 2; i++ {
		acceptCh := make(chan sessionResult, 1)
		go func() {
			sess, err := ln.Accept(ctx)
			acceptCh <- sessionResult{session: sess, err: err}
		}()

		conn, err := net.DialTCP("tcp", nil, tcpLn.Addr().(*net.TCPAddr))
		if err != nil {
			t.Fatalf("DialTCP error = %v, want nil", err)
		}
		clientSess, err := DialTLSSession(ctx, cfg, []connectivity.TCPConn{
			{Conn: conn, Origin: connectivity.TCPConnOriginDial},
		}, nil)
		if err != nil {
			t.Fatalf("DialTLSSession error = %v, want nil", err)
		}
		t.Cleanup(func() { _ = clientSess.Close(CloseReasonDaemonShutdown) })

		res := <-acceptCh
		if res.err != nil {
			_ = clientSess.Close(CloseReasonDaemonShutdown)
			t.Fatalf("listener.Accept(tls) error = %v, want nil", res.err)
		}
		t.Cleanup(func() { _ = res.session.Close(CloseReasonDaemonShutdown) })

		runShellPingStreams(t, ctx, clientSess, res.session, 1)

		_ = clientSess.Close(CloseReasonDaemonShutdown)
		_ = res.session.Close(CloseReasonDaemonShutdown)
	}
}

func TestPeerSessionListener_AcceptHonorsCancellation(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	serverUDP := listenLocalUDP(t)
	cfg := testSessionConfig(ProtocolQUIC, PathFamilyUDP4)

	ln, err := ListenSessions(context.Background(), cfg, serverUDP, nil)
	if err != nil {
		t.Fatalf("ListenSessions error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	start := time.Now()
	_, err = ln.Accept(ctx)
	if err == nil {
		t.Fatalf("Accept(cancelled ctx) error = nil, want error")
	}
	if time.Since(start) > time.Second {
		t.Fatalf("Accept(cancelled ctx) took too long: %v", time.Since(start))
	}
}
