package runtime

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocv1/enroll"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/presence"
	"github.com/miopunch/miopunch/internal/pocv1/punch"
	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
	"github.com/miopunch/miopunch/internal/shellproto"
	"github.com/miopunch/miopunch/internal/shelltarget"
	"github.com/miopunch/miopunch/internal/udpowner"
)

func TestSnapshotStageProgression(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() {
		rt.mu.Lock()
		rt.presence = nil
		rt.mu.Unlock()
		_ = rt.Close()
	})

	rt.mu.Lock()
	got := rt.derivedStageLocked()
	rt.mu.Unlock()
	if got != StageNetwork {
		t.Fatalf("derivedStageLocked() = %q, want %q", got, StageNetwork)
	}

	rt.mu.Lock()
	rt.meta.ActiveNetworkID = "net-1"
	rt.mu.Unlock()
	rt.mu.Lock()
	got = rt.derivedStageLocked()
	rt.mu.Unlock()
	if got != StageEnroll {
		t.Fatalf("joined derivedStageLocked() = %q, want %q", got, StageEnroll)
	}

	rt.mu.Lock()
	rt.presence = &presence.Session{}
	rt.mu.Unlock()
	rt.mu.Lock()
	got = rt.derivedStageLocked()
	rt.mu.Unlock()
	if got != StageDiscover {
		t.Fatalf("discover derivedStageLocked() = %q, want %q", got, StageDiscover)
	}

	rt.peerSessions.SetChangeHook(nil)
	rt.peerSessions.Put(&fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolQUIC,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC(),
		healthy:      true,
	})
	rt.mu.Lock()
	got = rt.derivedStageLocked()
	rt.mu.Unlock()
	if got != StageSecureSession {
		t.Fatalf("secure session derivedStageLocked() = %q, want %q", got, StageSecureSession)
	}

	rt.mu.Lock()
	rt.pingGate["peer-a"] = time.Now().UTC().UnixMilli()
	rt.mu.Unlock()
	rt.mu.Lock()
	got = rt.derivedStageLocked()
	rt.mu.Unlock()
	if got != StageShell {
		t.Fatalf("shell gate derivedStageLocked() = %q, want %q", got, StageShell)
	}
}

func TestSnapshotPreservesStatusEvidence(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	problem := newProblem(
		StageShell,
		poc.ReasonCodeUnavailable,
		poc.ExitCodeUnavailable,
		"shell gate rejected",
		[]poc.Fact{{Message: "peer_id=peer-a"}},
		[]poc.Suggestion{{Message: "retry after ping"}},
	)
	rt.setStatus(problem)

	snapshot := rt.Snapshot()
	if snapshot.Stage != StageShell {
		t.Fatalf("Snapshot().Stage = %q, want %q", snapshot.Stage, StageShell)
	}
	if snapshot.ReasonCode != poc.ReasonCodeUnavailable {
		t.Fatalf("Snapshot().ReasonCode = %q, want %q", snapshot.ReasonCode, poc.ReasonCodeUnavailable)
	}
	if snapshot.Summary.Text != "shell gate rejected" {
		t.Fatalf("Snapshot().Summary.Text = %q, want %q", snapshot.Summary.Text, "shell gate rejected")
	}
	if len(snapshot.Evidence.Facts) != 1 || snapshot.Evidence.Facts[0].Message != "peer_id=peer-a" {
		t.Fatalf("Snapshot().Evidence.Facts = %#v, want peer_id fact", snapshot.Evidence.Facts)
	}
	if len(snapshot.Evidence.Suggestions) != 1 || snapshot.Evidence.Suggestions[0].Message != "retry after ping" {
		t.Fatalf("Snapshot().Evidence.Suggestions = %#v, want retry suggestion", snapshot.Evidence.Suggestions)
	}
}

func TestPunchConfigUsesPersistedStunServers(t *testing.T) {
	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	networkID, err := pocwire.EncodeNetworkID(bytes.Repeat([]byte{0x44}, pocwire.RawIDLen))
	if err != nil {
		t.Fatalf("EncodeNetworkID() error = %v, want nil", err)
	}
	stunServers := []string{"stun1.example.net:3478", "stun2.example.net:3478"}
	if err := rt.store.PersistJoinedBootstrap(persist.JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: []byte("self-member-credential"),
		MailboxSecret:        bytes.Repeat([]byte{0x33}, 32),
		RuntimeBroker: persist.RuntimeBroker{
			Endpoint:    "broker.example.net:1883",
			StunServers: stunServers,
		},
		RosterSnapshot: persist.RosterSnapshot{},
	}); err != nil {
		t.Fatalf("PersistJoinedBootstrap() error = %v, want nil", err)
	}
	keys, err := rt.store.EnsureDeviceKeys()
	if err != nil {
		t.Fatalf("EnsureDeviceKeys() error = %v, want nil", err)
	}
	pub, err := keys.Ed25519PublicKey()
	if err != nil {
		t.Fatalf("Ed25519PublicKey() error = %v, want nil", err)
	}
	conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("net.ListenUDP() error = %v, want nil", err)
	}
	owner, err := udpowner.NewKCPOwner(conn, udpowner.KCPOwnerConfig{})
	if err != nil {
		t.Fatalf("NewKCPOwner() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = owner.Close() })

	rt.mu.Lock()
	rt.meta.ActiveNetworkID = networkID
	rt.meta.AuthorityEd25519PubB64 = encodeKeyB64(pub)
	rt.udpConn = conn
	rt.udpOwner = owner
	rt.mu.Unlock()

	cfg, problem := rt.punchConfig(peerPathPolicy{
		P2PNetwork:  connectivity.P2PNetworkUDPOnly,
		P2PIPFamily: connectivity.P2PIPFamilyV4,
	})
	if problem != nil {
		t.Fatalf("punchConfig() problem = %v, want nil", problem)
	}
	if strings.Join(cfg.StunServers, ",") != strings.Join(stunServers, ",") {
		t.Fatalf("punchConfig().StunServers = %#v, want %#v", cfg.StunServers, stunServers)
	}
	if cfg.P2PNetwork != connectivity.P2PNetworkUDPOnly {
		t.Fatalf("punchConfig().P2PNetwork = %q, want %q", cfg.P2PNetwork, connectivity.P2PNetworkUDPOnly)
	}
	if cfg.P2PIPFamily != connectivity.P2PIPFamilyV4 {
		t.Fatalf("punchConfig().P2PIPFamily = %q, want %q", cfg.P2PIPFamily, connectivity.P2PIPFamilyV4)
	}
}

func TestEnsurePeerSessionExplicitIPv4DoesNotReuseIPv6Session(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	oldSession := &fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolKCP,
			PathFamily:   dataplane.PathFamilyUDP6,
		},
		lastActivity: time.Now().UTC(),
		healthy:      true,
		pathFacts: dataplane.SessionPathFacts{
			SelectedPath: punch.PathDirectIPv6,
		},
	}
	rt.peerSessions.Put(oldSession)

	_, problem := rt.ensurePeerSession(context.Background(), "peer-a", peerPathPolicy{
		P2PNetwork:  connectivity.P2PNetworkAuto,
		P2PIPFamily: connectivity.P2PIPFamilyV4,
	})
	if problem == nil {
		t.Fatal("ensurePeerSession(ipv4 policy) problem = nil, want setup failure after closing incompatible session")
	}
	if oldSession.Healthy() {
		t.Fatal("ensurePeerSession(ipv4 policy) left old IPv6 session healthy, want closed")
	}
	if got := oldSession.CloseReason(); got != dataplane.CloseReasonSessionSuperseded {
		t.Fatalf("oldSession.CloseReason() = %q, want %q", got, dataplane.CloseReasonSessionSuperseded)
	}
}

func TestDoPingTCPOnlyFailsBeforeUDPEstablishment(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	_, problem := rt.doPing(context.Background(), PingArgs{
		PeerID:     "peer-a",
		P2PNetwork: string(connectivity.P2PNetworkTCPOnly),
	})
	if problem == nil {
		t.Fatal("doPing(tcp_only) problem = nil, want unsupported-path problem")
	}
	if problem.reasonCode != poc.ReasonCodeNotImplemented {
		t.Fatalf("doPing(tcp_only) reasonCode = %q, want %q", problem.reasonCode, poc.ReasonCodeNotImplemented)
	}
	if !hasFact(problem.facts, "p2p_network=tcp_only") {
		t.Fatalf("doPing(tcp_only) facts = %#v, want p2p_network fact", problem.facts)
	}
}

func TestIsPeerSessionTransportProblem(t *testing.T) {
	t.Parallel()

	cases := []struct {
		name    string
		problem *problem
		want    bool
	}{
		{
			name:    "shell stream open unavailable",
			problem: wrapProblem(StageShell, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to open shell stream", errors.New("closed"), "retry"),
			want:    true,
		},
		{
			name:    "shell control error is logical",
			problem: newProblem(StageShell, poc.ReasonCodeSHTargetNotFound, poc.ExitCodeNotFound, "target not found", nil, nil),
			want:    false,
		},
		{
			name:    "ping gate rejection is logical",
			problem: newProblem(StageShell, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "ping gate was rejected by the remote peer", nil, nil),
			want:    false,
		},
		{
			name:    "secure session unavailable",
			problem: wrapProblem(StageSecureSession, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to establish secure session", errors.New("closed"), "retry"),
			want:    true,
		},
	}

	for _, tc := range cases {
		tc := tc
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := isPeerSessionTransportProblem(tc.problem); got != tc.want {
				t.Fatalf("isPeerSessionTransportProblem(%q) = %t, want %t", tc.name, got, tc.want)
			}
		})
	}
}

func TestDoShell_PingGateRejectedStopsBeforeAttach(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })
	if _, problem := rt.doInitNetwork(context.Background(), InitNetworkArgs{}); problem != nil {
		t.Fatalf("doInitNetwork() problem = %v, want nil", problem)
	}

	var (
		mu        sync.Mutex
		openCount int
		remoteWG  sync.WaitGroup
		remoteErr = make(chan error, 1)
	)

	rt.peerSessions.Put(&fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolQUIC,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC(),
		healthy:      true,
		pathFacts: dataplane.SessionPathFacts{
			SelectedPath: punch.PathDirectIPv4,
		},
		openStream: func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
			mu.Lock()
			openCount++
			mu.Unlock()

			clientSide, remoteSide := net.Pipe()
			remoteWG.Add(1)
			go func() {
				defer remoteWG.Done()
				defer remoteSide.Close()

				var control shellproto.Control
				if err := shellproto.ReadJSON(remoteSide, &control); err != nil {
					remoteErr <- err
					return
				}
				if control.Op != shellproto.OpPing {
					remoteErr <- io.ErrUnexpectedEOF
					return
				}
				if err := shellproto.WriteJSON(remoteSide, shellproto.Control{
					Op: shellproto.OpPing,
					OK: false,
				}); err != nil {
					remoteErr <- err
				}
			}()
			return clientSide, nil
		},
	})

	result, problem := rt.doShell(context.Background(), ShellArgs{PeerID: "peer-a"})
	if problem == nil {
		t.Fatalf("doShell() problem = nil, want non-nil")
	}
	if problem.stage != StageShell {
		t.Fatalf("doShell() stage = %q, want %q", problem.stage, StageShell)
	}
	if problem.reasonCode != poc.ReasonCodeUnavailable {
		t.Fatalf("doShell() reasonCode = %q, want %q", problem.reasonCode, poc.ReasonCodeUnavailable)
	}
	if !hasFact(problem.facts, "selected_path="+punch.PathDirectIPv4) {
		t.Fatalf("doShell() facts = %#v, want selected_path fact", problem.facts)
	}
	if result.ShellSessionID != "" {
		t.Fatalf("doShell() shellSessionID = %q, want empty", result.ShellSessionID)
	}

	remoteWG.Wait()
	select {
	case err := <-remoteErr:
		t.Fatalf("remote shell control error = %v, want nil", err)
	default:
	}

	mu.Lock()
	gotOpenCount := openCount
	mu.Unlock()
	if gotOpenCount != 1 {
		t.Fatalf("doShell() stream open count = %d, want %d", gotOpenCount, 1)
	}
	if rt.hasPingGate("peer-a") {
		t.Fatalf("hasPingGate(peer-a) = true, want false")
	}
	if got := rt.Snapshot().Stage; got == StageShell {
		t.Fatalf("Snapshot().Stage = %q, want a non-shell stage", got)
	}
	if shells := rt.Snapshot().ShellSessions; len(shells) != 0 {
		t.Fatalf("Snapshot().ShellSessions = %#v, want empty", shells)
	}
}

func TestDoShellAttachFailureReportsSelectedPath(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	var openCount atomic.Int64
	rt.peerSessions.Put(&fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolKCP,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC(),
		healthy:      true,
		pathFacts: dataplane.SessionPathFacts{
			SelectedPath: punch.PathDirectIPv4,
		},
		openStream: func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
			if openCount.Add(1) == 2 {
				return nil, io.ErrClosedPipe
			}

			clientSide, remoteSide := net.Pipe()
			go func() {
				defer remoteSide.Close()
				var control shellproto.Control
				if err := shellproto.ReadJSON(remoteSide, &control); err != nil {
					return
				}
				_ = shellproto.WriteJSON(remoteSide, shellproto.Control{
					Op: control.Op,
					OK: true,
				})
			}()
			return clientSide, nil
		},
	})

	_, problem := rt.doShell(context.Background(), ShellArgs{PeerID: "peer-a"})
	if problem == nil {
		t.Fatal("doShell() problem = nil, want non-nil")
	}
	if !hasFact(problem.facts, "selected_path="+punch.PathDirectIPv4) {
		t.Fatalf("doShell() facts = %#v, want selected_path fact", problem.facts)
	}
	if got := openCount.Load(); got != 2 {
		t.Fatalf("doShell() stream open count = %d, want 2", got)
	}
}

func TestDoPingReportsSelectedPath(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	rt.peerSessions.Put(&fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolKCP,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC(),
		healthy:      true,
		pathFacts: dataplane.SessionPathFacts{
			SelectedPath: punch.PathDirectIPv4,
		},
		openStream: func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
			clientSide, remoteSide := net.Pipe()
			go func() {
				defer remoteSide.Close()
				var control shellproto.Control
				if err := shellproto.ReadJSON(remoteSide, &control); err != nil {
					return
				}
				_ = shellproto.WriteJSON(remoteSide, shellproto.Control{
					Op: control.Op,
					OK: true,
				})
			}()
			return clientSide, nil
		},
	})

	result, problem := rt.doPing(context.Background(), PingArgs{PeerID: "peer-a"})
	if problem != nil {
		t.Fatalf("doPing() problem = %v, want nil", problem)
	}
	if !hasFact(result.Evidence.Facts, "selected_path="+punch.PathDirectIPv4) {
		t.Fatalf("doPing() facts = %#v, want selected_path fact", result.Evidence.Facts)
	}
	if got := string(result.Data); !strings.Contains(got, `"selected_path":"`+punch.PathDirectIPv4+`"`) {
		t.Fatalf("doPing() data = %s, want selected_path", got)
	}
	if got := string(result.ReportMarkdown); !strings.Contains(got, "- selected_path="+punch.PathDirectIPv4) {
		t.Fatalf("doPing() report = %s, want selected_path fact", got)
	}
}

func TestDoPingFailureReportsSelectedPath(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	rt.peerSessions.Put(&fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolKCP,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC(),
		healthy:      true,
		pathFacts: dataplane.SessionPathFacts{
			SelectedPath: punch.PathDirectIPv4,
		},
		openStream: func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
			return nil, io.ErrClosedPipe
		},
	})

	_, problem := rt.doPing(context.Background(), PingArgs{PeerID: "peer-a"})
	if problem == nil {
		t.Fatal("doPing() problem = nil, want non-nil")
	}
	if !hasFact(problem.facts, "selected_path="+punch.PathDirectIPv4) {
		t.Fatalf("doPing() facts = %#v, want selected_path fact", problem.facts)
	}
}

func TestDoShellList_OutputsTargetsInFactsAndData(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })
	if _, problem := rt.doInitNetwork(context.Background(), InitNetworkArgs{}); problem != nil {
		t.Fatalf("doInitNetwork() problem = %v, want nil", problem)
	}

	rt.peerSessions.Put(&fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolQUIC,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC(),
		healthy:      true,
		pathFacts: dataplane.SessionPathFacts{
			SelectedPath: punch.PathPunchingIPv4,
		},
		openStream: func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
			clientSide, remoteSide := net.Pipe()
			go func() {
				defer remoteSide.Close()

				var control shellproto.Control
				if err := shellproto.ReadJSON(remoteSide, &control); err != nil {
					return
				}
				if control.Op != shellproto.OpShLS {
					return
				}
				_ = shellproto.WriteJSON(remoteSide, shellproto.Control{
					Op:      shellproto.OpShLS,
					OK:      true,
					Targets: []string{"ssh:git", "wsl:Debian"},
				})
			}()
			return clientSide, nil
		},
	})
	rt.markPingGate("peer-a")

	result, problem := rt.doShellList(context.Background(), ShellArgs{PeerID: "peer-a"})
	if problem != nil {
		t.Fatalf("doShellList() problem = %v, want nil", problem)
	}
	if !hasFact(result.Evidence.Facts, "peer_id=peer-a") {
		t.Fatalf("doShellList() facts = %#v, want peer fact", result.Evidence.Facts)
	}
	if !hasFact(result.Evidence.Facts, "selected_path="+punch.PathPunchingIPv4) {
		t.Fatalf("doShellList() facts = %#v, want selected_path fact", result.Evidence.Facts)
	}
	if !hasFact(result.Evidence.Facts, "target=ssh:git") {
		t.Fatalf("doShellList() facts = %#v, want ssh target fact", result.Evidence.Facts)
	}
	if !hasFact(result.Evidence.Facts, "target=wsl:Debian") {
		t.Fatalf("doShellList() facts = %#v, want wsl target fact", result.Evidence.Facts)
	}
	if got := string(result.Data); !strings.Contains(got, `"targets":["ssh:git","wsl:Debian"]`) {
		t.Fatalf("doShellList() data = %s, want targets array", got)
	}
	if got := string(result.Data); !strings.Contains(got, `"selected_path":"`+punch.PathPunchingIPv4+`"`) {
		t.Fatalf("doShellList() data = %s, want selected_path", got)
	}
	if got := string(result.ReportMarkdown); !strings.Contains(got, "- target=ssh:git") || !strings.Contains(got, "- target=wsl:Debian") {
		t.Fatalf("doShellList() report = %s, want target facts", got)
	}
	if got := string(result.ReportMarkdown); !strings.Contains(got, "- selected_path="+punch.PathPunchingIPv4) {
		t.Fatalf("doShellList() report = %s, want selected_path fact", got)
	}
	if len(result.Lines) != 2 || result.Lines[0] != "ssh:git" || result.Lines[1] != "wsl:Debian" {
		t.Fatalf("doShellList() lines = %#v, want target lines", result.Lines)
	}
}

func TestDoShellListFailureReportsSelectedPath(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	rt.peerSessions.Put(&fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolKCP,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC(),
		healthy:      true,
		pathFacts: dataplane.SessionPathFacts{
			SelectedPath: punch.PathPunchingIPv4,
		},
		openStream: func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
			return nil, io.ErrClosedPipe
		},
	})

	_, problem := rt.doShellList(context.Background(), ShellArgs{PeerID: "peer-a"})
	if problem == nil {
		t.Fatal("doShellList() problem = nil, want non-nil")
	}
	if !hasFact(problem.facts, "selected_path="+punch.PathPunchingIPv4) {
		t.Fatalf("doShellList() facts = %#v, want selected_path fact", problem.facts)
	}
}

func TestDoShellList_OutputsSessionsWhenTargetResolved(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })
	if _, problem := rt.doInitNetwork(context.Background(), InitNetworkArgs{}); problem != nil {
		t.Fatalf("doInitNetwork() problem = %v, want nil", problem)
	}

	rt.peerSessions.Put(&fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolQUIC,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC(),
		healthy:      true,
		openStream: func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
			clientSide, remoteSide := net.Pipe()
			go func() {
				defer remoteSide.Close()

				var control shellproto.Control
				if err := shellproto.ReadJSON(remoteSide, &control); err != nil {
					return
				}
				if control.Op != shellproto.OpShLS || control.Target != "wsl:Debian" {
					return
				}
				_ = shellproto.WriteJSON(remoteSide, shellproto.Control{
					Op:       shellproto.OpShLS,
					OK:       true,
					Target:   "wsl:Debian",
					Sessions: []string{"main", "recovery"},
				})
			}()
			return clientSide, nil
		},
	})
	rt.markPingGate("peer-a")

	result, problem := rt.doShellList(context.Background(), ShellArgs{PeerID: "peer-a", Target: "wsl:Debian"})
	if problem != nil {
		t.Fatalf("doShellList(target) problem = %v, want nil", problem)
	}
	if !hasFact(result.Evidence.Facts, "peer_id=peer-a") {
		t.Fatalf("doShellList(target) facts = %#v, want peer fact", result.Evidence.Facts)
	}
	if !hasFact(result.Evidence.Facts, "target=wsl:Debian") {
		t.Fatalf("doShellList(target) facts = %#v, want target fact", result.Evidence.Facts)
	}
	if !hasFact(result.Evidence.Facts, "session=main") {
		t.Fatalf("doShellList(target) facts = %#v, want session fact", result.Evidence.Facts)
	}
	if !hasFact(result.Evidence.Facts, "session=recovery") {
		t.Fatalf("doShellList(target) facts = %#v, want session fact", result.Evidence.Facts)
	}
	if got := string(result.Data); !strings.Contains(got, `"target":"wsl:Debian"`) || !strings.Contains(got, `"sessions":["main","recovery"]`) {
		t.Fatalf("doShellList(target) data = %s, want sessions array", got)
	}
	if got := string(result.ReportMarkdown); !strings.Contains(got, "- session=main") || !strings.Contains(got, "- session=recovery") {
		t.Fatalf("doShellList(target) report = %s, want session facts", got)
	}
	if len(result.Lines) != 2 || result.Lines[0] != "main" || result.Lines[1] != "recovery" {
		t.Fatalf("doShellList(target) lines = %#v, want session lines", result.Lines)
	}
}

func TestDoShellList_OutputsReadyTargetsAndStatuses(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })
	if _, problem := rt.doInitNetwork(context.Background(), InitNetworkArgs{}); problem != nil {
		t.Fatalf("doInitNetwork() problem = %v, want nil", problem)
	}

	rt.peerSessions.Put(&fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolQUIC,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC(),
		healthy:      true,
		openStream: func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
			clientSide, remoteSide := net.Pipe()
			go func() {
				defer remoteSide.Close()

				var control shellproto.Control
				if err := shellproto.ReadJSON(remoteSide, &control); err != nil {
					return
				}
				if control.Op != shellproto.OpShLS || !control.ReadyOnly || strings.TrimSpace(control.Target) != "" {
					return
				}
				_ = shellproto.WriteJSON(remoteSide, shellproto.Control{
					Op:        shellproto.OpShLS,
					OK:        true,
					ReadyOnly: true,
					Targets:   []string{"wsl:Debian"},
					TargetStatuses: []shellproto.TargetStatus{
						{Target: "wsl:Debian", Status: "ready"},
						{Target: "ssh:ale", Status: "unknown", ReasonCode: "UNAVAILABLE", Message: "Host key verification failed."},
						{Target: "ssh:ops", Status: "unsupported", ReasonCode: "SH_TMUX_MISSING", Message: "tmux missing"},
					},
				})
			}()
			return clientSide, nil
		},
	})
	rt.markPingGate("peer-a")

	result, problem := rt.doShellList(context.Background(), ShellArgs{PeerID: "peer-a", ReadyOnly: true})
	if problem != nil {
		t.Fatalf("doShellList(ready) problem = %v, want nil", problem)
	}
	if len(result.Lines) != 1 || result.Lines[0] != "wsl:Debian" {
		t.Fatalf("doShellList(ready) lines = %#v, want ready target only", result.Lines)
	}
	for _, fact := range []string{
		"target=wsl:Debian status=ready",
		"target=ssh:ale status=unknown reason_code=UNAVAILABLE",
		"target=ssh:ops status=unsupported reason_code=SH_TMUX_MISSING",
		"target_count=3",
		"ready_target_count=1",
		"unsupported_target_count=1",
		"unknown_target_count=1",
	} {
		if !hasFact(result.Evidence.Facts, fact) {
			t.Fatalf("doShellList(ready) facts missing %q: %#v", fact, result.Evidence.Facts)
		}
	}
	if got := string(result.Data); !strings.Contains(got, `"targets":["wsl:Debian"]`) || !strings.Contains(got, `"target_statuses":[`) {
		t.Fatalf("doShellList(ready) data = %s, want ready targets and target_statuses", got)
	}
	if got := string(result.ReportMarkdown); !strings.Contains(got, "- target=ssh:ale status=unknown reason_code=UNAVAILABLE") {
		t.Fatalf("doShellList(ready) report = %s, want target status facts", got)
	}
}

func TestProbeReadyTargetsRunsConcurrently(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	t.Cleanup(cancel)

	targets := []string{"ssh:ale", "wsl:Debian", "ssh:ops"}
	wslCalled := make(chan struct{}, 1)
	releaseSlow := make(chan struct{})

	probe := func(ctx context.Context, target string) (shelltarget.TargetReadiness, error) {
		switch target {
		case "ssh:ale", "ssh:ops":
			select {
			case <-releaseSlow:
			case <-ctx.Done():
				return shelltarget.TargetReadiness{}, ctx.Err()
			}
			return shelltarget.TargetReadiness{
				Target:     target,
				Status:     shelltarget.TargetStatusUnknown,
				ReasonCode: string(poc.ReasonCodeUnavailable),
				Message:    "slow probe",
			}, nil
		case "wsl:Debian":
			select {
			case wslCalled <- struct{}{}:
			default:
			}
			return shelltarget.TargetReadiness{
				Target: target,
				Status: shelltarget.TargetStatusReady,
			}, nil
		default:
			return shelltarget.TargetReadiness{}, errors.New("unexpected target: " + target)
		}
	}

	done := make(chan struct {
		readyTargets []string
		statuses     []shellproto.TargetStatus
	}, 1)
	go func() {
		readyTargets, statuses := probeReadyTargets(ctx, targets, probe)
		done <- struct {
			readyTargets []string
			statuses     []shellproto.TargetStatus
		}{
			readyTargets: readyTargets,
			statuses:     statuses,
		}
	}()

	select {
	case <-wslCalled:
	case <-time.After(100 * time.Millisecond):
		close(releaseSlow)
		t.Fatal("probeReadyTargets() did not start wsl:Debian before slow ssh probes finished")
	}

	close(releaseSlow)

	var result struct {
		readyTargets []string
		statuses     []shellproto.TargetStatus
	}
	select {
	case result = <-done:
	case <-time.After(time.Second):
		t.Fatal("probeReadyTargets() timed out waiting for results")
	}

	wantReadyTargets := []string{"wsl:Debian"}
	if len(result.readyTargets) != len(wantReadyTargets) || result.readyTargets[0] != wantReadyTargets[0] {
		t.Fatalf("probeReadyTargets() readyTargets = %#v, want %#v", result.readyTargets, wantReadyTargets)
	}

	wantStatuses := []shellproto.TargetStatus{
		{Target: "ssh:ale", Status: shelltarget.TargetStatusUnknown, ReasonCode: string(poc.ReasonCodeUnavailable), Message: "slow probe"},
		{Target: "wsl:Debian", Status: shelltarget.TargetStatusReady},
		{Target: "ssh:ops", Status: shelltarget.TargetStatusUnknown, ReasonCode: string(poc.ReasonCodeUnavailable), Message: "slow probe"},
	}
	if len(result.statuses) != len(wantStatuses) {
		t.Fatalf("probeReadyTargets() statuses = %#v, want %#v", result.statuses, wantStatuses)
	}
	for i := range wantStatuses {
		got := result.statuses[i]
		want := wantStatuses[i]
		if got.Target != want.Target || got.Status != want.Status || got.ReasonCode != want.ReasonCode || got.Message != want.Message {
			t.Fatalf("probeReadyTargets() statuses[%d] = %#v, want %#v", i, got, want)
		}
	}
}

func TestDoShellList_RejectsReadyWithConcreteTarget(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	_, problem := rt.doShellList(context.Background(), ShellArgs{
		PeerID:    "peer-a",
		Target:    "ssh:ale",
		ReadyOnly: true,
	})
	if problem == nil {
		t.Fatal("doShellList(ready+target) problem = nil, want non-nil")
	}
	if problem.reasonCode != poc.ReasonCodeBadRequest {
		t.Fatalf("doShellList(ready+target) reasonCode = %q, want %q", problem.reasonCode, poc.ReasonCodeBadRequest)
	}
	if problem.exitCode != poc.ExitCodeBadRequest {
		t.Fatalf("doShellList(ready+target) exitCode = %d, want %d", problem.exitCode, poc.ExitCodeBadRequest)
	}
}

func TestKeepaliveValidatedSessionsSkipsUnvalidatedSession(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	var openCount atomic.Int64
	sess := &fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolQUIC,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC().Add(-sessionKeepaliveMinIdle - time.Second),
		healthy:      true,
		openStream: func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
			openCount.Add(1)
			return nil, errors.New("unexpected keepalive attempt")
		},
	}
	rt.peerSessions.Put(sess)

	rt.keepaliveValidatedSessions()

	if got := openCount.Load(); got != 0 {
		t.Fatalf("keepaliveValidatedSessions() open count = %d, want 0", got)
	}
	if !sess.Healthy() {
		t.Fatal("keepaliveValidatedSessions() closed unvalidated session, want healthy")
	}
}

func TestKeepaliveValidatedSessionsKeepsValidatedSessionAlive(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	var openCount atomic.Int64
	sess := &fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolQUIC,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC().Add(-sessionKeepaliveMinIdle - time.Second),
		healthy:      true,
		openStream: func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
			openCount.Add(1)
			clientSide, remoteSide := net.Pipe()
			go func() {
				defer remoteSide.Close()
				var control shellproto.Control
				if err := shellproto.ReadJSON(remoteSide, &control); err != nil {
					return
				}
				_ = shellproto.WriteJSON(remoteSide, shellproto.Control{
					Op: control.Op,
					OK: true,
				})
			}()
			return clientSide, nil
		},
	}
	rt.peerSessions.Put(sess)
	rt.markPingGate("peer-a")

	rt.keepaliveValidatedSessions()

	if got := openCount.Load(); got != 1 {
		t.Fatalf("keepaliveValidatedSessions() open count = %d, want 1", got)
	}
	if !sess.Healthy() {
		t.Fatal("keepaliveValidatedSessions() healthy = false, want true")
	}
}

func TestKeepaliveValidatedSessionsClosesFailedSession(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	sess := &fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolQUIC,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC().Add(-sessionKeepaliveMinIdle - time.Second),
		healthy:      true,
		openStream: func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
			return nil, io.ErrClosedPipe
		},
	}
	rt.peerSessions.Put(sess)
	rt.markPingGate("peer-a")

	rt.keepaliveValidatedSessions()

	if _, ok := rt.peerSessions.Get(sess.key); ok {
		t.Fatal("keepaliveValidatedSessions() session still present, want closed")
	}
	if got := sess.CloseReason(); got != dataplane.CloseReasonTransportFatal {
		t.Fatalf("keepaliveValidatedSessions() close reason = %q, want %q", got, dataplane.CloseReasonTransportFatal)
	}
}

func TestPunchProblemIncludesDiagnosticFacts(t *testing.T) {
	t.Parallel()

	err := &punch.Error{Diagnostic: punch.Diagnostic{
		DialID:             "dial-1",
		RemotePeerID:       "peer-a",
		LocalCandidates:    []punch.Candidate{{Kind: punch.CandidateKindHost, Addr: "127.0.0.1:4001"}},
		RemoteCandidates:   []punch.Candidate{{Kind: punch.CandidateKindHost, Addr: "127.0.0.1:5001"}},
		PlannedPairCount:   1,
		AttemptConcurrency: 2,
		AttemptBudget:      time.Second,
		AttemptedPairs: []punch.AttemptEvidence{
			{
				LocalCandidate:  punch.Candidate{Kind: punch.CandidateKindHost, Addr: "127.0.0.1:4001"},
				RemoteCandidate: punch.Candidate{Kind: punch.CandidateKindHost, Addr: "127.0.0.1:5001"},
				Path:            punch.PathDirectIPv4,
				Result:          "timeout",
				Detail:          "deadline exceeded",
			},
		},
	}, Err: context.DeadlineExceeded}

	problem := punchProblem("failed to establish punched path", "peer-a", err)
	if problem.stage != StagePunch {
		t.Fatalf("punchProblem().stage = %q, want %q", problem.stage, StagePunch)
	}
	if problem.reasonCode != poc.ReasonCodeTimeout {
		t.Fatalf("punchProblem().reasonCode = %q, want %q", problem.reasonCode, poc.ReasonCodeTimeout)
	}
	if !hasFact(problem.facts, "planned_pair_count=1") {
		t.Fatalf("punchProblem().facts = %#v, want planned_pair_count fact", problem.facts)
	}
	if !hasFact(problem.facts, "attempt_results=timeout=1") {
		t.Fatalf("punchProblem().facts = %#v, want attempt_results fact", problem.facts)
	}
	if !hasFact(problem.facts, "attempt_paths="+punch.PathDirectIPv4+"=1") {
		t.Fatalf("punchProblem().facts = %#v, want attempt_paths fact", problem.facts)
	}
	wantDetails := "attempt_details=direct_ipv4:timeout:127.0.0.1:4001->127.0.0.1:5001:deadline exceeded"
	if !hasFact(problem.facts, wantDetails) {
		t.Fatalf("punchProblem().facts = %#v, want %q", problem.facts, wantDetails)
	}
}

func TestAppendPathResultProblemFactsPreservesSelectedPath(t *testing.T) {
	t.Parallel()

	problem := wrapProblem(
		StageSecureSession,
		poc.ReasonCodeUnavailable,
		poc.ExitCodeUnavailable,
		"failed to establish secure session",
		errors.New("dial failed"),
		"retry",
	)
	result := punch.PathResult{
		RemoteAddr: &net.UDPAddr{IP: net.IPv4(127, 0, 0, 2), Port: 5002},
		Evidence: punch.PunchEvidence{
			SelectedPath:      punch.PathDirectIPv4,
			SelectedLocal:     punch.Candidate{Kind: punch.CandidateKindHost, Addr: "127.0.0.1:4001"},
			SelectedRemote:    punch.Candidate{Kind: punch.CandidateKindHost, Addr: "127.0.0.2:5002"},
			SelectedRemoteUDP: "127.0.0.2:5002",
		},
	}

	got := appendPathResultProblemFacts(problem, result)
	for _, fact := range []string{
		"selected_path=" + punch.PathDirectIPv4,
		"selected_local_candidate=host@127.0.0.1:4001",
		"selected_remote_candidate=host@127.0.0.2:5002",
		"selected_remote_udp=127.0.0.2:5002",
	} {
		if !hasFact(got.facts, fact) {
			t.Fatalf("appendPathResultProblemFacts() facts = %#v, want %q", got.facts, fact)
		}
	}
}

func hasFact(facts []poc.Fact, want string) bool {
	for _, fact := range facts {
		if fact.Message == want {
			return true
		}
	}
	return false
}

func hasFactPrefix(facts []poc.Fact, prefix string) bool {
	for _, fact := range facts {
		if strings.HasPrefix(fact.Message, prefix) {
			return true
		}
	}
	return false
}

func mustFactValue(t *testing.T, facts []poc.Fact, key string) string {
	t.Helper()
	prefix := key + "="
	for _, fact := range facts {
		if strings.HasPrefix(fact.Message, prefix) {
			return strings.TrimPrefix(fact.Message, prefix)
		}
	}
	t.Fatalf("facts = %#v, want key %q", facts, key)
	return ""
}

func TestLocalCandidatesForPortPrefersNonLoopbackIPv4(t *testing.T) {
	t.Parallel()

	got := localCandidatesForPort(4242, []localInterfaceAddr{
		{
			Name:  "lo",
			Flags: net.FlagUp | net.FlagLoopback,
			Addr:  &net.IPNet{IP: net.ParseIP("127.0.0.1"), Mask: net.CIDRMask(8, 32)},
		},
		{
			Name:  "eth0",
			Flags: net.FlagUp,
			Addr:  &net.IPNet{IP: net.ParseIP("172.25.0.4"), Mask: net.CIDRMask(16, 32)},
		},
		{
			Name:  "eth0",
			Flags: net.FlagUp,
			Addr:  &net.IPNet{IP: net.ParseIP("::1"), Mask: net.CIDRMask(128, 128)},
		},
	})
	if len(got) != 1 {
		t.Fatalf("localCandidatesForPort() length = %d, want 1", len(got))
	}
	if got[0].Addr != "172.25.0.4:4242" {
		t.Fatalf("localCandidatesForPort() addr = %q, want %q", got[0].Addr, "172.25.0.4:4242")
	}
}

func TestLocalCandidatesForPortFallsBackToLoopback(t *testing.T) {
	t.Parallel()

	got := localCandidatesForPort(4242, []localInterfaceAddr{
		{
			Name:  "lo",
			Flags: net.FlagUp | net.FlagLoopback,
			Addr:  &net.IPNet{IP: net.ParseIP("127.0.0.1"), Mask: net.CIDRMask(8, 32)},
		},
		{
			Name:  "lo",
			Flags: net.FlagUp | net.FlagLoopback,
			Addr:  &net.IPNet{IP: net.ParseIP("::1"), Mask: net.CIDRMask(128, 128)},
		},
	})
	if len(got) != 1 {
		t.Fatalf("localCandidatesForPort() length = %d, want 1", len(got))
	}
	if got[0].Addr != "127.0.0.1:4242" {
		t.Fatalf("localCandidatesForPort() addr = %q, want %q", got[0].Addr, "127.0.0.1:4242")
	}
}

func TestLocalCandidatesForPortCanSkipLoopbackFallback(t *testing.T) {
	t.Parallel()

	got := localCandidatesForPortWithFallback(4242, []localInterfaceAddr{
		{
			Name:  "lo",
			Flags: net.FlagUp | net.FlagLoopback,
			Addr:  &net.IPNet{IP: net.ParseIP("127.0.0.1"), Mask: net.CIDRMask(8, 32)},
		},
	}, false)
	if len(got) != 0 {
		t.Fatalf("localCandidatesForPortWithFallback(allowLoopbackFallback=false) = %#v, want empty", got)
	}
}

func TestLocalCandidatesForPortKeepsVirtualNamesAndFiltersLoopbackLinkLocal(t *testing.T) {
	t.Parallel()

	got := localCandidatesForPort(4242, []localInterfaceAddr{
		{
			Name:  "lo",
			Flags: net.FlagUp | net.FlagLoopback,
			Addr:  &net.IPNet{IP: net.ParseIP("10.255.255.254"), Mask: net.CIDRMask(32, 32)},
		},
		{
			Name:  "eth1",
			Flags: net.FlagUp,
			Addr:  &net.IPNet{IP: net.ParseIP("169.254.83.107"), Mask: net.CIDRMask(16, 32)},
		},
		{
			Name:  "docker0",
			Flags: net.FlagUp,
			Addr:  &net.IPNet{IP: net.ParseIP("172.17.0.1"), Mask: net.CIDRMask(16, 32)},
		},
		{
			Name:  "br-cecf21e17fe9",
			Flags: net.FlagUp,
			Addr:  &net.IPNet{IP: net.ParseIP("172.18.0.1"), Mask: net.CIDRMask(16, 32)},
		},
		{
			Name:  "vEthernet (Default Switch)",
			Flags: net.FlagUp,
			Addr:  &net.IPNet{IP: net.ParseIP("192.168.144.1"), Mask: net.CIDRMask(20, 32)},
		},
		{
			Name:  "eth2",
			Flags: net.FlagUp,
			Addr:  &net.IPNet{IP: net.ParseIP("192.168.4.5"), Mask: net.CIDRMask(24, 32)},
		},
	})
	want := []string{
		"172.17.0.1:4242",
		"172.18.0.1:4242",
		"192.168.144.1:4242",
		"192.168.4.5:4242",
	}
	if len(got) != len(want) {
		t.Fatalf("localCandidatesForPort() length = %d, want %d: %#v", len(got), len(want), got)
	}
	for i, candidate := range got {
		if candidate.Addr != want[i] {
			t.Fatalf("localCandidatesForPort()[%d].Addr = %q, want %q", i, candidate.Addr, want[i])
		}
	}
}

func TestOpenAppliesBrokerOverride(t *testing.T) {
	t.Parallel()

	root := t.TempDir()
	rt, err := Open(Options{Root: root, BrokerURL: "broker.example:1883"})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	if got := rt.currentBrokerEndpoint(); got != "tcp://broker.example:1883" {
		t.Fatalf("currentBrokerEndpoint() = %q, want %q", got, "tcp://broker.example:1883")
	}
	rt.mu.Lock()
	got := rt.meta.RuntimeBrokerOverride
	rt.mu.Unlock()
	if got != "tcp://broker.example:1883" {
		t.Fatalf("rt.meta.RuntimeBrokerOverride = %q, want %q", got, "tcp://broker.example:1883")
	}
}

func TestOpenUsesDefaultRuntimeBrokerWhenUnconfigured(t *testing.T) {
	t.Parallel()

	root := t.TempDir()
	rt, err := Open(Options{Root: root})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	if got := rt.currentBrokerEndpoint(); got != defaultRuntimeBrokerEndpoint {
		t.Fatalf("currentBrokerEndpoint() = %q, want %q", got, defaultRuntimeBrokerEndpoint)
	}
	rt.mu.Lock()
	got := rt.meta.RuntimeBrokerOverride
	rt.mu.Unlock()
	if got != defaultRuntimeBrokerEndpoint {
		t.Fatalf("rt.meta.RuntimeBrokerOverride = %q, want %q", got, defaultRuntimeBrokerEndpoint)
	}
}

func TestOpenKeepsJoinedBrokerWhenUnconfigured(t *testing.T) {
	t.Parallel()

	root := t.TempDir()
	meta := metadata{
		ActiveNetworkID: "joined-network",
		Role:            "admin",
		BrokerEndpoint:  "tcp://persisted.example:1883",
	}
	if err := saveMetadata(root, meta); err != nil {
		t.Fatalf("saveMetadata() error = %v, want nil", err)
	}
	rt, err := Open(Options{Root: root})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	if got := rt.currentBrokerEndpoint(); got != meta.BrokerEndpoint {
		t.Fatalf("currentBrokerEndpoint() = %q, want %q", got, meta.BrokerEndpoint)
	}
	rt.mu.Lock()
	got := rt.meta.RuntimeBrokerOverride
	rt.mu.Unlock()
	if got != "" {
		t.Fatalf("rt.meta.RuntimeBrokerOverride = %q, want empty", got)
	}
}

func TestDoInitNetworkUsesDefaultRuntimeBrokerWithoutEmbeddedBroker(t *testing.T) {
	t.Parallel()

	root := t.TempDir()
	rt, err := Open(Options{Root: root})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	_, problem := rt.doInitNetwork(ctx, InitNetworkArgs{})
	if problem != nil {
		if problem.stage != StageEnroll || problem.reasonCode != poc.ReasonCodeUnavailable {
			t.Fatalf("doInitNetwork() problem = %v, want runtime-workers unavailable", problem)
		}
	}

	rt.mu.Lock()
	networkID := rt.meta.ActiveNetworkID
	currentEmbeddedBroker := rt.broker
	currentBrokerOverride := rt.meta.RuntimeBrokerOverride
	currentBrokerEndpoint := rt.meta.BrokerEndpoint
	rt.mu.Unlock()
	if networkID == "" {
		t.Fatalf("rt.meta.ActiveNetworkID = empty, want non-empty")
	}
	if currentEmbeddedBroker != nil {
		t.Fatalf("rt.broker = %#v, want nil when using default runtime broker", currentEmbeddedBroker)
	}
	if currentBrokerOverride != defaultRuntimeBrokerEndpoint {
		t.Fatalf("rt.meta.RuntimeBrokerOverride = %q, want %q", currentBrokerOverride, defaultRuntimeBrokerEndpoint)
	}
	if currentBrokerEndpoint != defaultRuntimeBrokerEndpoint {
		t.Fatalf("rt.meta.BrokerEndpoint = %q, want %q", currentBrokerEndpoint, defaultRuntimeBrokerEndpoint)
	}
	broker, err := rt.store.LoadRuntimeBroker(networkID)
	if err != nil {
		t.Fatalf("LoadRuntimeBroker() error = %v, want nil", err)
	}
	if broker.Endpoint != defaultRuntimeBrokerEndpoint {
		t.Fatalf("LoadRuntimeBroker().Endpoint = %q, want %q", broker.Endpoint, defaultRuntimeBrokerEndpoint)
	}
}

func TestDoInitNetworkUsesBrokerOverrideWithoutEmbeddedBroker(t *testing.T) {
	t.Parallel()

	externalBroker, err := startEmbeddedBroker("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("startEmbeddedBroker() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = externalBroker.Close() })

	root := t.TempDir()
	rt, err := Open(Options{Root: root, BrokerURL: externalBroker.Endpoint()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	_, problem := rt.doInitNetwork(context.Background(), InitNetworkArgs{})
	if problem != nil {
		if problem.stage != StageEnroll || problem.reasonCode != poc.ReasonCodeUnavailable {
			t.Fatalf("doInitNetwork() problem = %v, want runtime-workers unavailable", problem)
		}
	}
	if got := rt.currentBrokerEndpoint(); got != externalBroker.Endpoint() {
		t.Fatalf("currentBrokerEndpoint() = %q, want %q", got, externalBroker.Endpoint())
	}
	rt.mu.Lock()
	networkID := rt.meta.ActiveNetworkID
	currentEmbeddedBroker := rt.broker
	currentBrokerOverride := rt.meta.RuntimeBrokerOverride
	rt.mu.Unlock()
	if networkID == "" {
		t.Fatalf("rt.meta.ActiveNetworkID = empty, want non-empty")
	}
	if currentEmbeddedBroker != nil {
		t.Fatalf("rt.broker = %#v, want nil when using external override", currentEmbeddedBroker)
	}
	if currentBrokerOverride != externalBroker.Endpoint() {
		t.Fatalf("rt.meta.RuntimeBrokerOverride = %q, want %q", currentBrokerOverride, externalBroker.Endpoint())
	}
	broker, err := rt.store.LoadRuntimeBroker(networkID)
	if err != nil {
		t.Fatalf("LoadRuntimeBroker() error = %v, want nil", err)
	}
	if broker.Endpoint != externalBroker.Endpoint() {
		t.Fatalf("LoadRuntimeBroker().Endpoint = %q, want %q", broker.Endpoint, externalBroker.Endpoint())
	}
}

func TestEnsureWorkersSkipsEmbeddedBrokerWhenOverridePresent(t *testing.T) {
	t.Parallel()

	externalBroker, err := startEmbeddedBroker("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("startEmbeddedBroker() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = externalBroker.Close() })

	root := t.TempDir()
	rt, err := Open(Options{Root: root, BrokerURL: externalBroker.Endpoint()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() {
		rt.mu.Lock()
		rt.presence = nil
		rt.udpConn = nil
		rt.mu.Unlock()
		_ = rt.Close()
	})

	_, problem := rt.doInitNetwork(context.Background(), InitNetworkArgs{})
	if problem != nil {
		if problem.stage != StageEnroll || problem.reasonCode != poc.ReasonCodeUnavailable {
			t.Fatalf("doInitNetwork() problem = %v, want runtime-workers unavailable", problem)
		}
	}

	rt.mu.Lock()
	rt.presence = nil
	if rt.udpConn != nil {
		_ = rt.udpConn.Close()
		rt.udpConn = nil
	}
	rt.broker = nil
	rt.mu.Unlock()

	if err := rt.ensureWorkers(context.Background()); err != nil {
		t.Fatalf("ensureWorkers() error = %v, want nil", err)
	}

	rt.mu.Lock()
	gotBroker := rt.broker
	gotOverride := rt.meta.RuntimeBrokerOverride
	rt.mu.Unlock()
	if gotBroker != nil {
		t.Fatalf("rt.broker = %#v, want nil when override is present", gotBroker)
	}
	if gotOverride != externalBroker.Endpoint() {
		t.Fatalf("rt.meta.RuntimeBrokerOverride = %q, want %q", gotOverride, externalBroker.Endpoint())
	}
}

func TestRefreshPresenceRosterProjectsNewlyApprovedPeer(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	externalBroker, err := startEmbeddedBroker("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("startEmbeddedBroker() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = externalBroker.Close() })

	rt, err := Open(Options{Root: t.TempDir(), BrokerURL: externalBroker.Endpoint()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	if _, problem := rt.doInitNetwork(ctx, InitNetworkArgs{}); problem != nil {
		t.Fatalf("doInitNetwork() problem = %v, want nil", problem)
	}

	rt.mu.Lock()
	networkID := rt.meta.ActiveNetworkID
	rt.mu.Unlock()
	if networkID == "" {
		t.Fatal("rt.meta.ActiveNetworkID = empty, want non-empty")
	}

	localKeys, err := rt.store.LoadDeviceKeys()
	if err != nil {
		t.Fatalf("LoadDeviceKeys(local) error = %v, want nil", err)
	}
	localPriv, err := localKeys.Ed25519PrivateKey()
	if err != nil {
		t.Fatalf("Ed25519PrivateKey(local) error = %v, want nil", err)
	}

	remoteStore, err := persist.Open(t.TempDir())
	if err != nil {
		t.Fatalf("persist.Open(remote) error = %v, want nil", err)
	}
	remoteKeys, err := remoteStore.EnsureDeviceKeys()
	if err != nil {
		t.Fatalf("EnsureDeviceKeys(remote) error = %v, want nil", err)
	}
	remotePeerID, err := remoteKeys.PeerID()
	if err != nil {
		t.Fatalf("PeerID(remote) error = %v, want nil", err)
	}
	remotePub, err := remoteKeys.Ed25519PublicKey()
	if err != nil {
		t.Fatalf("Ed25519PublicKey(remote) error = %v, want nil", err)
	}
	remoteX25519Pub, err := remoteKeys.X25519PublicKey()
	if err != nil {
		t.Fatalf("X25519PublicKey(remote) error = %v, want nil", err)
	}

	remoteCredential := enroll.MemberCredential{
		NetworkID:         networkID,
		SubjectEd25519Pub: append([]byte(nil), remotePub...),
		SubjectX25519Pub:  append([]byte(nil), remoteX25519Pub...),
		Role:              "member",
		NotBeforeUnixMs:   uint64(time.Now().UTC().UnixMilli()),
		NotAfterUnixMs:    uint64(time.Now().Add(time.Hour).UTC().UnixMilli()),
		IssuerKeyID:       "authority",
	}
	if err := enroll.SignMemberCredential(localPriv, &remoteCredential); err != nil {
		t.Fatalf("SignMemberCredential(remote) error = %v, want nil", err)
	}
	remoteCredentialBytes, err := remoteCredential.MarshalBinary()
	if err != nil {
		t.Fatalf("MarshalBinary(remote) error = %v, want nil", err)
	}

	adminCredentialBytes, err := rt.store.LoadSelfMemberCredential(networkID)
	if err != nil {
		t.Fatalf("LoadSelfMemberCredential(admin) error = %v, want nil", err)
	}
	mailboxSecret, err := rt.store.LoadMailboxSecret(networkID)
	if err != nil {
		t.Fatalf("LoadMailboxSecret(admin) error = %v, want nil", err)
	}
	roster, err := rt.store.LoadRosterSnapshot(networkID)
	if err != nil {
		t.Fatalf("LoadRosterSnapshot(admin) error = %v, want nil", err)
	}
	roster.Entries = append(roster.Entries, persist.RosterEntry{
		PeerID:           remotePeerID,
		MemberCredential: remoteCredentialBytes,
		DeviceName:       "remote",
		Platform:         "windows",
	})
	if err := rt.store.ReplaceRosterSnapshot(networkID, roster); err != nil {
		t.Fatalf("ReplaceRosterSnapshot(admin) error = %v, want nil", err)
	}

	if err := remoteStore.PersistJoinedBootstrap(persist.JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: remoteCredentialBytes,
		MailboxSecret:        append([]byte(nil), mailboxSecret...),
		RuntimeBroker:        persist.RuntimeBroker{Endpoint: externalBroker.Endpoint()},
		RosterSnapshot: persist.RosterSnapshot{
			Entries: []persist.RosterEntry{
				{
					PeerID:           roster.Entries[0].PeerID,
					MemberCredential: adminCredentialBytes,
					DeviceName:       roster.Entries[0].DeviceName,
					Platform:         roster.Entries[0].Platform,
				},
			},
		},
	}); err != nil {
		t.Fatalf("PersistJoinedBootstrap(remote) error = %v, want nil", err)
	}

	remoteCfg, err := presence.LoadConfig(remoteStore, networkID, "remote", "windows", "1.0.0")
	if err != nil {
		t.Fatalf("LoadConfig(remote) error = %v, want nil", err)
	}
	remoteSession, err := presence.OpenSession(ctx, remoteCfg)
	if err != nil {
		t.Fatalf("OpenSession(remote) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = remoteSession.Abort() })

	before := rt.Snapshot().DiscoverView
	for _, peer := range before.Peers {
		if peer.PeerID == remotePeerID {
			t.Fatalf("before refresh discover view already contained remote peer: %#v", before.Peers)
		}
	}

	if err := rt.refreshPresenceRoster(ctx); err != nil {
		t.Fatalf("refreshPresenceRoster() error = %v, want nil", err)
	}

	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		for _, peer := range rt.Snapshot().DiscoverView.Peers {
			if peer.PeerID == remotePeerID && peer.OnlineState == presence.OnlineStateOnline {
				return
			}
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("Snapshot().DiscoverView = %#v, want remote peer %q online", rt.Snapshot().DiscoverView.Peers, remotePeerID)
}

func TestApproveRefreshesPresenceRosterForNewPeer(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	externalBroker, err := startEmbeddedBroker("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("startEmbeddedBroker() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = externalBroker.Close() })

	adminRT, err := Open(Options{Root: t.TempDir(), BrokerURL: externalBroker.Endpoint()})
	if err != nil {
		t.Fatalf("Open(admin) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = adminRT.Close() })

	if _, initProblem := adminRT.doInitNetwork(ctx, InitNetworkArgs{}); initProblem != nil {
		t.Fatalf("doInitNetwork(admin) problem = %v, want nil", initProblem)
	}

	adminRT.mu.Lock()
	networkID := adminRT.meta.ActiveNetworkID
	adminRT.mu.Unlock()
	if networkID == "" {
		t.Fatal("adminRT.meta.ActiveNetworkID = empty, want non-empty")
	}

	inviteResult, inviteProblem := adminRT.doInvite(ctx, InviteArgs{Mode: "approve"})
	if inviteProblem != nil {
		t.Fatalf("doInvite(admin) problem = %v, want nil", inviteProblem)
	}

	var inviteCode string
	for _, fact := range inviteResult.Evidence.Facts {
		if strings.HasPrefix(fact.Message, "invite_code=") {
			inviteCode = strings.TrimPrefix(fact.Message, "invite_code=")
			break
		}
	}
	if inviteCode == "" {
		t.Fatalf("doInvite(admin) facts = %#v, want invite_code", inviteResult.Evidence.Facts)
	}

	memberRT, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open(member) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = memberRT.Close() })

	approveDone := make(chan ActionResult, 1)
	approveErr := make(chan *problem, 1)
	go func() {
		result, problem := adminRT.doApprove(ctx, ApproveArgs{Code: inviteCode})
		if problem != nil {
			approveErr <- problem
			return
		}
		approveDone <- result
	}()

	joinResult, joinProblem := memberRT.doJoin(ctx, JoinArgs{Code: inviteCode})
	if joinProblem != nil {
		t.Fatalf("doJoin(member) problem = %v, want nil", joinProblem)
	}

	select {
	case approveProblem := <-approveErr:
		t.Fatalf("doApprove(admin) problem = %v, want nil", approveProblem)
	case <-approveDone:
	case <-ctx.Done():
		t.Fatalf("doApprove(admin) timed out: %v", ctx.Err())
	}

	var joinedPeerID string
	for _, fact := range joinResult.Evidence.Facts {
		if strings.HasPrefix(fact.Message, "peer_id=") {
			joinedPeerID = strings.TrimPrefix(fact.Message, "peer_id=")
			break
		}
	}
	if joinedPeerID == "" {
		t.Fatalf("doJoin(member) facts = %#v, want peer_id", joinResult.Evidence.Facts)
	}

	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		snapshot := adminRT.Snapshot().DiscoverView
		for _, peer := range snapshot.Peers {
			if peer.PeerID == joinedPeerID && peer.OnlineState == presence.OnlineStateOnline {
				return
			}
		}
		time.Sleep(20 * time.Millisecond)
	}

	adminRoster, err := adminRT.store.LoadRosterSnapshot(networkID)
	if err != nil {
		t.Fatalf("LoadRosterSnapshot(admin) error = %v, want nil", err)
	}
	rosterJSON, err := json.Marshal(adminRoster)
	if err != nil {
		t.Fatalf("json.Marshal(adminRoster) error = %v, want nil", err)
	}
	t.Fatalf("adminRT.Snapshot().DiscoverView = %#v, want joined peer %q online; roster=%s", adminRT.Snapshot().DiscoverView.Peers, joinedPeerID, string(rosterJSON))
}

func TestDoJoinTimeoutIncludesBrokerAndTopicFacts(t *testing.T) {
	t.Parallel()

	adminRT, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open(admin) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = adminRT.Close() })

	if _, initProblem := adminRT.doInitNetwork(context.Background(), InitNetworkArgs{}); initProblem != nil {
		t.Fatalf("doInitNetwork(admin) problem = %v, want nil", initProblem)
	}

	inviteResult, inviteProblem := adminRT.doInvite(context.Background(), InviteArgs{})
	if inviteProblem != nil {
		t.Fatalf("doInvite(admin) problem = %v, want nil", inviteProblem)
	}

	inviteCode := mustFactValue(t, inviteResult.Evidence.Facts, "invite_code")
	memberRT, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open(member) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = memberRT.Close() })

	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()

	joinResult, joinProblem := memberRT.Action(ctx, "join", mustJSONMarshal(JoinArgs{Code: inviteCode}))
	if joinProblem == nil {
		t.Fatal("Action(join) problem = nil, want timeout problem")
	}
	if joinProblem.reasonCode != poc.ReasonCodeTimeout && joinProblem.reasonCode != poc.ReasonCodeUnavailable {
		t.Fatalf("Action(join) reasonCode = %q, want %q or %q", joinProblem.reasonCode, poc.ReasonCodeTimeout, poc.ReasonCodeUnavailable)
	}
	requireFacts := []string{
		"invite_id=" + mustFactValue(t, inviteResult.Evidence.Facts, "invite_id"),
		"network_id=" + mustFactValue(t, inviteResult.Evidence.Facts, "network_id"),
		"join_topic=" + mustFactValue(t, inviteResult.Evidence.Facts, "join_topic"),
		"broker_endpoint=" + mustFactValue(t, inviteResult.Evidence.Facts, "broker_endpoint"),
		"peer_id=",
	}
	switch {
	case strings.Contains(joinResult.Summary.Text, "failed to open join signaling session"):
	case strings.Contains(joinResult.Summary.Text, "failed to publish join request"),
		strings.Contains(joinResult.Summary.Text, "timed out waiting for enroll response"):
		requireFacts = append(requireFacts, "reply_topic=")
	default:
		t.Fatalf("Action(join) Summary.Text = %q, want signaling-stage join failure", joinResult.Summary.Text)
	}
	for _, fact := range requireFacts {
		if strings.HasSuffix(fact, "=") {
			if !hasFactPrefix(joinResult.Evidence.Facts, fact) {
				t.Fatalf("Action(join) facts = %#v, want fact prefix %q", joinResult.Evidence.Facts, fact)
			}
			continue
		}
		if !hasFact(joinResult.Evidence.Facts, fact) {
			t.Fatalf("Action(join) facts = %#v, want %q", joinResult.Evidence.Facts, fact)
		}
	}
}

type fakePeerSession struct {
	key          dataplane.SessionKey
	lastActivity time.Time
	healthy      bool
	pathFacts    dataplane.SessionPathFacts
	openStream   func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error)
	closeReason  dataplane.CloseReason
	mu           sync.Mutex
}

func (s *fakePeerSession) Key() dataplane.SessionKey {
	return s.key
}

func (s *fakePeerSession) SessionPathFacts() dataplane.SessionPathFacts {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.pathFacts.Normalize()
}

func (s *fakePeerSession) OpenStream(ctx context.Context, open dataplane.StreamOpen) (io.ReadWriteCloser, error) {
	if s.openStream != nil {
		return s.openStream(ctx, open)
	}
	return nil, nil
}

func (s *fakePeerSession) AcceptStream(context.Context) (*dataplane.AcceptedStream, error) {
	return nil, nil
}

func (s *fakePeerSession) Close(reason dataplane.CloseReason) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.healthy = false
	s.closeReason = reason
	return nil
}

func (s *fakePeerSession) CloseReason() dataplane.CloseReason {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.closeReason
}

func (s *fakePeerSession) Healthy() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.healthy
}

func (s *fakePeerSession) LastActivity() time.Time {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.lastActivity
}
