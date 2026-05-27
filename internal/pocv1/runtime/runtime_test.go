package runtime

import (
	"context"
	"encoding/json"
	"io"
	"net"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocv1/enroll"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/presence"
	"github.com/miopunch/miopunch/internal/pocv1/punch"
	"github.com/miopunch/miopunch/internal/shellproto"
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
	rt.peerSessions.Put(fakePeerSession{
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

func TestDoShell_PingGateRejectedStopsBeforeAttach(t *testing.T) {
	t.Parallel()

	rt, err := Open(Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = rt.Close() })

	var (
		mu        sync.Mutex
		openCount int
		remoteWG  sync.WaitGroup
		remoteErr = make(chan error, 1)
	)

	rt.peerSessions.Put(fakePeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: "peer-a",
			Protocol:     dataplane.ProtocolQUIC,
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		lastActivity: time.Now().UTC(),
		healthy:      true,
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
			{LocalCandidate: punch.Candidate{Kind: punch.CandidateKindHost, Addr: "127.0.0.1:4001"}, RemoteCandidate: punch.Candidate{Kind: punch.CandidateKindHost, Addr: "127.0.0.1:5001"}, Result: "timeout", Detail: "deadline exceeded"},
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
}

func hasFact(facts []poc.Fact, want string) bool {
	for _, fact := range facts {
		if fact.Message == want {
			return true
		}
	}
	return false
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

func TestLocalCandidatesForPortFiltersVirtualAndLinkLocalAddrs(t *testing.T) {
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
	if len(got) != 1 {
		t.Fatalf("localCandidatesForPort() length = %d, want 1", len(got))
	}
	if got[0].Addr != "192.168.4.5:4242" {
		t.Fatalf("localCandidatesForPort() addr = %q, want %q", got[0].Addr, "192.168.4.5:4242")
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

type fakePeerSession struct {
	key          dataplane.SessionKey
	lastActivity time.Time
	healthy      bool
	openStream   func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error)
}

func (s fakePeerSession) Key() dataplane.SessionKey {
	return s.key
}

func (s fakePeerSession) OpenStream(ctx context.Context, open dataplane.StreamOpen) (io.ReadWriteCloser, error) {
	if s.openStream != nil {
		return s.openStream(ctx, open)
	}
	return nil, nil
}

func (s fakePeerSession) AcceptStream(context.Context) (*dataplane.AcceptedStream, error) {
	return nil, nil
}

func (s fakePeerSession) Close(dataplane.CloseReason) error {
	return nil
}

func (s fakePeerSession) CloseReason() dataplane.CloseReason {
	return ""
}

func (s fakePeerSession) Healthy() bool {
	return s.healthy
}

func (s fakePeerSession) LastActivity() time.Time {
	return s.lastActivity
}
