package punch

import (
	"context"
	"errors"
	"net"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/punchdecision"
	"github.com/miopunch/miopunch/internal/punching"
	"github.com/miopunch/miopunch/internal/udpowner"
	legacywire "github.com/miopunch/miopunch/internal/wire"
)

func TestAllowedRemoteUDPAddrsForDirectIPv6IncludesObservedAndPeerCandidates(t *testing.T) {
	resp := &legacywire.NatHoleResp{
		PeerDirectAddrs: []string{
			"[fd00::2]:4000",
			"127.0.0.1:4000",
			"bad",
			"[fd00::3]:4000",
		},
	}
	attemptRes := &connectivity.AttemptResult{
		Path:   PathDirectIPv6,
		Remote: mustUDPAddr(t, "[fd00::9]:4000"),
	}

	got := allowedRemoteUDPAddrsForAttempt(resp, attemptRes)
	want := []string{
		"[fd00::9]:4000",
		"[fd00::2]:4000",
		"[fd00::3]:4000",
	}
	if len(got) != len(want) {
		t.Fatalf("allowedRemoteUDPAddrsForAttempt() length = %d (%v), want %d", len(got), got, len(want))
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("allowedRemoteUDPAddrsForAttempt()[%d] = %q, want %q (full=%v)", i, got[i], want[i], got)
		}
	}
}

func TestAllowedRemoteUDPAddrsForAttemptKeepsNonIPv6Exact(t *testing.T) {
	resp := &legacywire.NatHoleResp{
		PeerDirectAddrs: []string{"[fd00::2]:4000"},
	}
	attemptRes := &connectivity.AttemptResult{
		Path:   PathPunchingIPv4,
		Remote: mustUDPAddr(t, "127.0.0.1:4000"),
	}

	got := allowedRemoteUDPAddrsForAttempt(resp, attemptRes)
	if len(got) != 1 || got[0] != "127.0.0.1:4000" {
		t.Fatalf("allowedRemoteUDPAddrsForAttempt() = %v, want [127.0.0.1:4000]", got)
	}
}

func TestExecutePairPlansHonorsConcurrencyAndSelectsWinner(t *testing.T) {
	var inFlight atomic.Int64
	var maxInFlight atomic.Int64

	cfg := LoadedConfig{
		AttemptConcurrency: 2,
		AttemptPair: func(ctx context.Context, demux *udpowner.TraversalDemux, plan pairPlan, key []byte) (AttemptPairResult, error) {
			cur := inFlight.Add(1)
			defer inFlight.Add(-1)
			for {
				prev := maxInFlight.Load()
				if cur <= prev || maxInFlight.CompareAndSwap(prev, cur) {
					break
				}
			}
			time.Sleep(20 * time.Millisecond)
			if plan.index == 2 {
				return AttemptPairResult{RemoteAddr: &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 5002}, Path: PathPunchingIPv4}, nil
			}
			return AttemptPairResult{}, context.DeadlineExceeded
		},
	}
	plans := []pairPlan{
		{index: 0, local: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:1"}, remote: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:2"}},
		{index: 1, local: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:1"}, remote: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:3"}},
		{index: 2, local: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:1"}, remote: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:4"}},
	}

	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	selected, evidence, err := executePairPlans(ctx, cfg, nil, plans, []byte("0123456789abcdef"))
	if err != nil {
		t.Fatalf("executePairPlans() error = %v, want nil", err)
	}
	if selected.RemoteAddr == nil || selected.RemoteAddr.Port != 5002 {
		t.Fatalf("executePairPlans().RemoteAddr = %#v, want winner port 5002", selected.RemoteAddr)
	}
	if got := maxInFlight.Load(); got > 2 {
		t.Fatalf("executePairPlans() max concurrency = %d, want <= 2", got)
	}
	if evidence[2].Result != "selected" {
		t.Fatalf("executePairPlans() winner result = %q, want %q", evidence[2].Result, "selected")
	}
	if evidence[2].Path != PathPunchingIPv4 {
		t.Fatalf("executePairPlans() winner path = %q, want %q", evidence[2].Path, PathPunchingIPv4)
	}
}

func TestExecutePairPlansAggregatesTimeoutWhenNoWinner(t *testing.T) {
	cfg := LoadedConfig{
		AttemptConcurrency: 1,
		AttemptPair: func(ctx context.Context, demux *udpowner.TraversalDemux, plan pairPlan, key []byte) (AttemptPairResult, error) {
			return AttemptPairResult{}, context.DeadlineExceeded
		},
	}
	plans := []pairPlan{
		{index: 0, local: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:1"}, remote: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:2"}},
	}
	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()
	_, evidence, err := executePairPlans(ctx, cfg, nil, plans, []byte("0123456789abcdef"))
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("executePairPlans() error = %v, want %v", err, context.DeadlineExceeded)
	}
	if evidence[0].Result != "timeout" {
		t.Fatalf("executePairPlans() evidence[0].Result = %q, want %q", evidence[0].Result, "timeout")
	}
}

func TestExecutePairPlansReturnsCallerCancellation(t *testing.T) {
	cfg := LoadedConfig{
		AttemptConcurrency: 1,
		AttemptPair: func(ctx context.Context, demux *udpowner.TraversalDemux, plan pairPlan, key []byte) (AttemptPairResult, error) {
			<-ctx.Done()
			return AttemptPairResult{}, ctx.Err()
		},
	}
	plans := []pairPlan{
		{index: 0, local: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:1"}, remote: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:2"}},
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	_, evidence, err := executePairPlans(ctx, cfg, nil, plans, []byte("0123456789abcdef"))
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("executePairPlans() error = %v, want %v", err, context.Canceled)
	}
	if evidence[0].Result != "canceled" {
		t.Fatalf("executePairPlans() evidence[0].Result = %q, want %q", evidence[0].Result, "canceled")
	}
}

func TestRunPunchReturnsSelectedPathEvidence(t *testing.T) {
	conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("net.ListenUDP() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = conn.Close() })

	remoteConn := mustListenUDPForIP(t, net.IPv4(127, 0, 0, 1))
	t.Cleanup(func() { _ = remoteConn.Close() })

	localCandidate := Candidate{Kind: CandidateKindHost, Addr: conn.LocalAddr().String()}
	remoteCandidate := Candidate{Kind: CandidateKindHost, Addr: remoteConn.LocalAddr().String()}
	dialID := mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA")
	key := []byte("0123456789abcdef")
	startDirectResponder(t, remoteConn, dialID, key)
	owner, err := udpowner.NewKCPOwner(conn, udpowner.KCPOwnerConfig{})
	if err != nil {
		t.Fatalf("NewKCPOwner() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = owner.Close() })

	cfg := LoadedConfig{
		UDPConn:            conn,
		UDPOwner:           owner,
		LocalCandidates:    []Candidate{localCandidate},
		AttemptConcurrency: 1,
		AttemptBudget:      time.Second,
	}
	remote := trustedRemote{
		PeerID:           mustCanonicalPeerID(t, "AAAAAAAAAAAAAAAAAAAAAAAAAA"),
		MemberCredential: []byte{0x01, 0x02, 0x03},
	}
	resp := &legacywire.NatHoleResp{
		TransactionID:   dialID,
		Sid:             dialID,
		P2PNetwork:      "udp_only",
		PeerDirectAddrs: []string{remoteCandidate.Addr},
		PunchingEnabled: false,
	}

	got, err := runPunch(
		context.Background(),
		cfg,
		remote,
		dialID,
		key,
		resp,
		UDPDecision{AnalyzerKey: "test", Mode: 0, Index: 0, LocalResponse: *resp, RemoteResponse: *resp},
		true,
	)
	if err != nil {
		t.Fatalf("runPunch() error = %v, want nil", err)
	}
	if got.Ownership() != SelectedUDPOwnershipRuntime {
		t.Fatalf("runPunch().Ownership() = %q, want %q", got.Ownership(), SelectedUDPOwnershipRuntime)
	}
	if got.RuntimeKCPPacket == nil {
		t.Fatalf("runPunch().RuntimeKCPPacket = nil, want owner-backed packet conn")
	}
	if got.RemoteAddr == nil || got.RemoteAddr.String() != remoteCandidate.Addr {
		t.Fatalf("runPunch().RemoteAddr = %#v, want %q", got.RemoteAddr, remoteCandidate.Addr)
	}
	if got.RemoteIdentity.PeerID != remote.PeerID {
		t.Fatalf("runPunch().RemoteIdentity.PeerID = %q, want %q", got.RemoteIdentity.PeerID, remote.PeerID)
	}
	if got.Evidence.SelectedLocal != localCandidate {
		t.Fatalf("runPunch().Evidence.SelectedLocal = %#v, want %#v", got.Evidence.SelectedLocal, localCandidate)
	}
	if got.Evidence.SelectedRemote != remoteCandidate {
		t.Fatalf("runPunch().Evidence.SelectedRemote = %#v, want %#v", got.Evidence.SelectedRemote, remoteCandidate)
	}
	if got.Evidence.SelectedRemoteUDP != remoteCandidate.Addr {
		t.Fatalf("runPunch().Evidence.SelectedRemoteUDP = %q, want %q", got.Evidence.SelectedRemoteUDP, remoteCandidate.Addr)
	}
	if got.Evidence.SelectedPath != PathDirectIPv4 {
		t.Fatalf("runPunch().Evidence.SelectedPath = %q, want %q", got.Evidence.SelectedPath, PathDirectIPv4)
	}
	if len(got.Evidence.AttemptedPairs) != 1 {
		t.Fatalf("runPunch().Evidence.AttemptedPairs length = %d, want 1", len(got.Evidence.AttemptedPairs))
	}
	if got.Evidence.AttemptedPairs[0].Result != "selected" {
		t.Fatalf("runPunch().Evidence.AttemptedPairs[0].Result = %q, want %q", got.Evidence.AttemptedPairs[0].Result, "selected")
	}
	if got.Evidence.AttemptedPairs[0].Path != PathDirectIPv4 {
		t.Fatalf("runPunch().Evidence.AttemptedPairs[0].Path = %q, want %q", got.Evidence.AttemptedPairs[0].Path, PathDirectIPv4)
	}
}

func TestRunPunchPreservesDirectFailureEvidenceWhenPunchingWins(t *testing.T) {
	conn := mustListenUDPForIP(t, net.IPv4(127, 0, 0, 1))
	t.Cleanup(func() { _ = conn.Close() })

	remoteConn := mustListenUDPForIP(t, net.IPv4(127, 0, 0, 1))
	t.Cleanup(func() { _ = remoteConn.Close() })

	localCandidate := Candidate{Kind: CandidateKindHost, Addr: conn.LocalAddr().String()}
	directCandidate := "192.0.2.10:4100"
	punchCandidate := remoteConn.LocalAddr().String()
	dialID := mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA")
	key := []byte("0123456789abcdef")

	cfg := LoadedConfig{
		UDPConn:            conn,
		LocalCandidates:    []Candidate{localCandidate},
		AttemptConcurrency: 1,
		AttemptBudget:      time.Second,
		AttemptUDP: func(ctx context.Context, sid string, key []byte, udp4Conn *net.UDPConn, udp6Conn *net.UDPConn, resp *legacywire.NatHoleResp, udp4Demux *udpowner.TraversalDemux, udp6Demux *udpowner.TraversalDemux) (*connectivity.AttemptResult, error) {
			_ = udp6Conn
			_ = udp4Demux
			_ = udp6Demux
			return &connectivity.AttemptResult{
				Path:   PathPunchingIPv4,
				Conn:   udp4Conn,
				Remote: remoteConn.LocalAddr().(*net.UDPAddr),
				Evidence: []connectivity.AttemptEvidence{
					{Path: PathDirectIPv4, Candidate: directCandidate, Result: "timeout", Detail: "context deadline exceeded"},
					{Path: PathPunchingIPv4, Candidate: punchCandidate, Result: "selected", Detail: punchCandidate},
				},
			}, nil
		},
	}
	remote := trustedRemote{
		PeerID:           mustCanonicalPeerID(t, "AAAAAAAAAAAAAAAAAAAAAAAAAA"),
		MemberCredential: []byte{0x01, 0x02, 0x03},
	}
	resp := &legacywire.NatHoleResp{
		TransactionID:   dialID,
		Sid:             dialID,
		P2PNetwork:      "udp_only",
		PeerDirectAddrs: []string{directCandidate},
		CandidateAddrs:  []string{punchCandidate},
		PunchingEnabled: true,
	}

	got, err := runPunch(
		context.Background(),
		cfg,
		remote,
		dialID,
		key,
		resp,
		UDPDecision{AnalyzerKey: "test", Mode: 2, Index: 1, LocalResponse: *resp, RemoteResponse: *resp},
		true,
	)
	if err != nil {
		t.Fatalf("runPunch() error = %v, want nil", err)
	}
	if got.Evidence.SelectedPath != PathPunchingIPv4 {
		t.Fatalf("runPunch().Evidence.SelectedPath = %q, want %q", got.Evidence.SelectedPath, PathPunchingIPv4)
	}
	if len(got.Evidence.AttemptedPairs) != 2 {
		t.Fatalf("runPunch().Evidence.AttemptedPairs length = %d, want 2", len(got.Evidence.AttemptedPairs))
	}
	if got.Evidence.AttemptedPairs[0].Path != PathDirectIPv4 || got.Evidence.AttemptedPairs[0].Result != "timeout" {
		t.Fatalf("direct evidence = %#v, want direct timeout", got.Evidence.AttemptedPairs[0])
	}
	if got.Evidence.AttemptedPairs[0].RemoteCandidate.Addr != directCandidate {
		t.Fatalf("direct evidence remote candidate = %#v, want %q", got.Evidence.AttemptedPairs[0].RemoteCandidate, directCandidate)
	}
	if got.Evidence.AttemptedPairs[1].Path != PathPunchingIPv4 || got.Evidence.AttemptedPairs[1].Result != "selected" {
		t.Fatalf("punching evidence = %#v, want punching selected", got.Evidence.AttemptedPairs[1])
	}
}

func TestRunPunchMarksTemporaryWinnerAndCloseReleasesIt(t *testing.T) {
	conn := mustListenUDPForIP(t, net.IPv4(127, 0, 0, 1))
	t.Cleanup(func() { _ = conn.Close() })
	owner, err := udpowner.NewKCPOwner(conn, udpowner.KCPOwnerConfig{})
	if err != nil {
		t.Fatalf("NewKCPOwner() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = owner.Close() })

	tempConn := mustListenUDPForIP(t, net.IPv4(127, 0, 0, 1))
	remoteConn := mustListenUDPForIP(t, net.IPv4(127, 0, 0, 1))
	t.Cleanup(func() { _ = remoteConn.Close() })

	localCandidate := Candidate{Kind: CandidateKindHost, Addr: conn.LocalAddr().String()}
	dialID := mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA")
	key := []byte("0123456789abcdef")
	cfg := LoadedConfig{
		UDPConn:            conn,
		UDPOwner:           owner,
		LocalCandidates:    []Candidate{localCandidate},
		AttemptConcurrency: 1,
		AttemptBudget:      time.Second,
		AttemptUDP: func(ctx context.Context, sid string, key []byte, udp4Conn *net.UDPConn, udp6Conn *net.UDPConn, resp *legacywire.NatHoleResp, udp4Demux *udpowner.TraversalDemux, udp6Demux *udpowner.TraversalDemux) (*connectivity.AttemptResult, error) {
			_ = ctx
			_ = sid
			_ = key
			_ = udp4Conn
			_ = udp6Conn
			_ = resp
			_ = udp4Demux
			_ = udp6Demux
			return &connectivity.AttemptResult{
				Path:   PathPunchingIPv4,
				Conn:   tempConn,
				Remote: remoteConn.LocalAddr().(*net.UDPAddr),
			}, nil
		},
	}
	remote := trustedRemote{
		PeerID:           mustCanonicalPeerID(t, "AAAAAAAAAAAAAAAAAAAAAAAAAA"),
		MemberCredential: []byte{0x01, 0x02, 0x03},
	}
	resp := &legacywire.NatHoleResp{
		TransactionID:   dialID,
		Sid:             dialID,
		P2PNetwork:      "udp_only",
		CandidateAddrs:  []string{remoteConn.LocalAddr().String()},
		PunchingEnabled: true,
	}

	got, err := runPunch(
		context.Background(),
		cfg,
		remote,
		dialID,
		key,
		resp,
		UDPDecision{AnalysisKey: "analysis", AnalyzerKey: "test", Mode: 2, Index: 1, LocalResponse: *resp, RemoteResponse: *resp},
		true,
	)
	if err != nil {
		t.Fatalf("runPunch() error = %v, want nil", err)
	}
	if got.Ownership() != SelectedUDPOwnershipTemporary {
		t.Fatalf("runPunch().Ownership() = %q, want %q", got.Ownership(), SelectedUDPOwnershipTemporary)
	}
	if got.Conn != tempConn {
		t.Fatalf("runPunch().Conn = %v, want temporary winner %v", got.Conn, tempConn)
	}
	if err := got.Close(); err != nil {
		t.Fatalf("PathResult.Close() error = %v, want nil", err)
	}
	_, err = tempConn.WriteToUDP([]byte("closed"), remoteConn.LocalAddr().(*net.UDPAddr))
	if err == nil {
		t.Fatalf("temporary winner WriteToUDP() error = nil, want closed error")
	}
}

func TestLocalUDPAnalyzerKeyUsesLocalRemotePeerScope(t *testing.T) {
	decision := UDPDecision{
		AnalysisKey: "analysis-key",
		AnalyzerKey: "responder-scoped-key",
	}

	got := localUDPAnalyzerKey("target-peer", decision)
	want := punchdecision.UDPAnalyzerKey("target-peer", "analysis-key")
	if got != want {
		t.Fatalf("localUDPAnalyzerKey() = %q, want %q", got, want)
	}
	if got == decision.AnalyzerKey {
		t.Fatalf("localUDPAnalyzerKey() reused remote AnalyzerKey %q", got)
	}
}

func TestRunPunchPreservesUDP6DirectRuntimeOwnership(t *testing.T) {
	udp4Conn := mustListenUDPForIP(t, net.IPv4(127, 0, 0, 1))
	t.Cleanup(func() { _ = udp4Conn.Close() })
	udp4Owner, err := udpowner.NewKCPOwner(udp4Conn, udpowner.KCPOwnerConfig{})
	if err != nil {
		t.Fatalf("NewKCPOwner(udp4) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = udp4Owner.Close() })

	udp6Addr := &net.UDPAddr{IP: net.ParseIP("::1"), Port: 0}
	udp6Conn, err := net.ListenUDP("udp6", udp6Addr)
	if err != nil {
		t.Skipf("udp6 unavailable: %v", err)
	}
	udp6Owner, err := udpowner.NewKCPOwner(udp6Conn, udpowner.KCPOwnerConfig{})
	if err != nil {
		_ = udp6Conn.Close()
		t.Fatalf("NewKCPOwner(udp6) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = udp6Owner.Close() })

	remote6Conn, err := net.ListenUDP("udp6", udp6Addr)
	if err != nil {
		t.Skipf("remote udp6 unavailable: %v", err)
	}
	t.Cleanup(func() { _ = remote6Conn.Close() })

	localCandidate := Candidate{Kind: CandidateKindHost, Addr: udp4Conn.LocalAddr().String()}
	dialID := mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA")
	key := []byte("0123456789abcdef")
	cfg := LoadedConfig{
		UDPConn:            udp4Conn,
		UDPOwner:           udp4Owner,
		UDP6Conn:           udp6Conn,
		UDP6Owner:          udp6Owner,
		LocalCandidates:    []Candidate{localCandidate},
		AttemptConcurrency: 1,
		AttemptBudget:      time.Second,
		AttemptUDP: func(ctx context.Context, sid string, key []byte, udp4Conn *net.UDPConn, udp6Conn *net.UDPConn, resp *legacywire.NatHoleResp, udp4Demux *udpowner.TraversalDemux, udp6Demux *udpowner.TraversalDemux) (*connectivity.AttemptResult, error) {
			_ = ctx
			_ = sid
			_ = key
			_ = udp4Conn
			_ = resp
			_ = udp4Demux
			if udp6Conn == nil || udp6Demux == nil {
				return nil, errors.New("udp6 attempt inputs missing")
			}
			return &connectivity.AttemptResult{
				Path:   PathDirectIPv6,
				Conn:   udp6Conn,
				Remote: remote6Conn.LocalAddr().(*net.UDPAddr),
			}, nil
		},
	}
	remote := trustedRemote{
		PeerID:           mustCanonicalPeerID(t, "AAAAAAAAAAAAAAAAAAAAAAAAAA"),
		MemberCredential: []byte{0x01, 0x02, 0x03},
	}
	resp := &legacywire.NatHoleResp{
		TransactionID:   dialID,
		Sid:             dialID,
		P2PNetwork:      "udp_only",
		PeerDirectAddrs: []string{remote6Conn.LocalAddr().String()},
	}

	got, err := runPunch(
		context.Background(),
		cfg,
		remote,
		dialID,
		key,
		resp,
		UDPDecision{AnalysisKey: "analysis", AnalyzerKey: "test", Mode: 0, Index: 0, LocalResponse: *resp, RemoteResponse: *resp},
		true,
	)
	if err != nil {
		t.Fatalf("runPunch() error = %v, want nil", err)
	}
	if got.Evidence.SelectedPath != PathDirectIPv6 {
		t.Fatalf("runPunch().Evidence.SelectedPath = %q, want %q", got.Evidence.SelectedPath, PathDirectIPv6)
	}
	if got.Ownership() != SelectedUDPOwnershipRuntime {
		t.Fatalf("runPunch().Ownership() = %q, want %q", got.Ownership(), SelectedUDPOwnershipRuntime)
	}
	if got.RuntimeKCPPacket == nil {
		t.Fatalf("runPunch().RuntimeKCPPacket = nil, want udp6 owner packet conn")
	}
}

func TestRunPunchUsesSymmetricPairSID(t *testing.T) {
	localA := Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:4001"}
	remoteA := Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:5001"}
	remoteB := Candidate{Kind: CandidateKindHost, Addr: "172.25.0.3:5001"}

	gotAB := sidForDialPair("dial-1", localA, remoteB)
	gotBA := sidForDialPair("dial-1", remoteB, localA)
	if gotAB != gotBA {
		t.Fatalf("sidForDialPair() symmetry mismatch: gotAB=%q gotBA=%q", gotAB, gotBA)
	}

	if sidForDialPair("dial-1", localA, remoteA) == gotAB {
		t.Fatalf("sidForDialPair() collision between distinct pairs")
	}
}

func TestMirroredHostRemoteAddrReturnsRemoteUDPAddr(t *testing.T) {
	plan := pairPlan{
		local:  Candidate{Kind: CandidateKindHost, Addr: "192.168.4.5:4001"},
		remote: Candidate{Kind: CandidateKindHost, Addr: "192.168.4.5:5001"},
	}

	got, ok, err := mirroredHostRemoteAddr(plan)
	if err != nil {
		t.Fatalf("mirroredHostRemoteAddr() error = %v, want nil", err)
	}
	if !ok {
		t.Fatal("mirroredHostRemoteAddr() ok = false, want true")
	}
	if got == nil || got.String() != "192.168.4.5:5001" {
		t.Fatalf("mirroredHostRemoteAddr() = %#v, want 192.168.4.5:5001", got)
	}
}

func TestMirroredHostRemoteAddrRejectsDifferentIPs(t *testing.T) {
	plan := pairPlan{
		local:  Candidate{Kind: CandidateKindHost, Addr: "192.168.4.5:4001"},
		remote: Candidate{Kind: CandidateKindHost, Addr: "192.168.4.6:5001"},
	}

	got, ok, err := mirroredHostRemoteAddr(plan)
	if err != nil {
		t.Fatalf("mirroredHostRemoteAddr() error = %v, want nil", err)
	}
	if ok {
		t.Fatal("mirroredHostRemoteAddr() ok = true, want false")
	}
	if got != nil {
		t.Fatalf("mirroredHostRemoteAddr() = %#v, want nil", got)
	}
}

func TestDefaultAttemptPairDirectIPv4Succeeds(t *testing.T) {
	key := []byte("0123456789abcdef")
	connA := mustListenUDPForIP(t, net.IPv4(127, 0, 0, 1))
	connB := mustListenUDPForIP(t, net.IPv4(127, 0, 0, 2))

	demuxA, err := udpowner.NewUDPTraversalDemux(connA, udpowner.DemuxConfig{Key: key})
	if err != nil {
		t.Fatalf("NewUDPTraversalDemux(A) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = demuxA.Close() })
	demuxB, err := udpowner.NewUDPTraversalDemux(connB, udpowner.DemuxConfig{Key: key})
	if err != nil {
		t.Fatalf("NewUDPTraversalDemux(B) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = demuxB.Close() })

	candA := Candidate{Kind: CandidateKindHost, Addr: connA.LocalAddr().String()}
	candB := Candidate{Kind: CandidateKindHost, Addr: connB.LocalAddr().String()}
	sid := "direct-test-sid"
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	type attemptOutcome struct {
		result AttemptPairResult
		err    error
	}
	resultA := make(chan attemptOutcome, 1)
	resultB := make(chan attemptOutcome, 1)
	failMakeHole := func(context.Context, *net.UDPConn, *udpowner.TraversalDemux, *legacywire.NatHoleResp, []byte) (*net.UDPConn, *net.UDPAddr, error) {
		return nil, nil, errors.New("unexpected punching fallback")
	}

	go func() {
		result, err := attemptPairWithPunch(ctx, demuxA, pairPlan{
			local: candA, remote: candB, sid: sid, conn: connA, resp: natHoleRespForPair(candB, sid, true),
		}, key, failMakeHole)
		resultA <- attemptOutcome{result: result, err: err}
	}()
	go func() {
		result, err := attemptPairWithPunch(ctx, demuxB, pairPlan{
			local: candB, remote: candA, sid: sid, conn: connB, resp: natHoleRespForPair(candA, sid, false),
		}, key, failMakeHole)
		resultB <- attemptOutcome{result: result, err: err}
	}()

	assertDirectOutcome := func(label string, ch <-chan attemptOutcome, wantRemote string) {
		t.Helper()
		select {
		case got := <-ch:
			if got.err != nil {
				t.Fatalf("%s attemptPairWithPunch() error = %v, want nil", label, got.err)
			}
			if got.result.Path != PathDirectIPv4 {
				t.Fatalf("%s attemptPairWithPunch().Path = %q, want %q", label, got.result.Path, PathDirectIPv4)
			}
			if got.result.RemoteAddr == nil || got.result.RemoteAddr.String() != wantRemote {
				t.Fatalf("%s attemptPairWithPunch().RemoteAddr = %#v, want %s", label, got.result.RemoteAddr, wantRemote)
			}
		case <-ctx.Done():
			t.Fatalf("%s attemptPairWithPunch() timed out: %v", label, ctx.Err())
		}
	}

	assertDirectOutcome("A", resultA, connB.LocalAddr().String())
	assertDirectOutcome("B", resultB, connA.LocalAddr().String())
}

func TestAttemptPairFallsBackToPunchingAfterDirectTimeout(t *testing.T) {
	key := []byte("0123456789abcdef")
	conn := mustListenUDPForIP(t, net.IPv4(127, 0, 0, 1))
	demux, err := udpowner.NewUDPTraversalDemux(conn, udpowner.DemuxConfig{Key: key})
	if err != nil {
		t.Fatalf("NewUDPTraversalDemux() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = demux.Close() })

	local := Candidate{Kind: CandidateKindHost, Addr: conn.LocalAddr().String()}
	remote := Candidate{Kind: CandidateKindHost, Addr: "127.0.0.2:49999"}
	remoteAddr := mustUDPAddr(t, remote.Addr)
	sid := "direct-timeout-fallback"
	var makeHoleCalled atomic.Int64
	makeHole := func(context.Context, *net.UDPConn, *udpowner.TraversalDemux, *legacywire.NatHoleResp, []byte) (*net.UDPConn, *net.UDPAddr, error) {
		makeHoleCalled.Add(1)
		return conn, remoteAddr, nil
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	got, err := attemptPairWithPunch(ctx, demux, pairPlan{
		local: local, remote: remote, sid: sid, conn: conn, resp: natHoleRespForPair(remote, sid, true),
	}, key, makeHole)
	if err != nil {
		t.Fatalf("attemptPairWithPunch() error = %v, want nil", err)
	}
	if got.Path != PathPunchingIPv4 {
		t.Fatalf("attemptPairWithPunch().Path = %q, want %q", got.Path, PathPunchingIPv4)
	}
	if makeHoleCalled.Load() != 1 {
		t.Fatalf("makeHole call count = %d, want 1", makeHoleCalled.Load())
	}
	if !strings.Contains(got.Detail, "direct_ipv4=timeout") {
		t.Fatalf("attemptPairWithPunch().Detail = %q, want direct timeout evidence", got.Detail)
	}
}

func mustListenUDPForIP(t *testing.T, ip net.IP) *net.UDPConn {
	t.Helper()
	conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: ip, Port: 0})
	if err != nil {
		t.Fatalf("net.ListenUDP(%s) error = %v, want nil", ip, err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	return conn
}

func startDirectResponder(t *testing.T, conn *net.UDPConn, sid string, key []byte) {
	t.Helper()
	demux, err := udpowner.NewUDPTraversalDemux(conn, udpowner.DemuxConfig{Key: key})
	if err != nil {
		t.Fatalf("NewUDPTraversalDemux() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = demux.Close() })
	ep := demux.Open(sid, 8)
	t.Cleanup(func() { _ = ep.Close() })

	done := make(chan struct{})
	t.Cleanup(func() { <-done })
	go func() {
		defer close(done)
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		buf := make([]byte, 2048)
		n, raddr, err := ep.Recv(ctx, buf)
		if err != nil {
			return
		}
		var msg legacywire.NatHoleSid
		if err := punching.DecodeMessageInto(buf[:n], key, &msg); err != nil {
			return
		}
		msg.Response = true
		payload, err := punching.EncodeMessage(&msg, key)
		if err != nil {
			return
		}
		_ = ep.SendTo(ctx, payload, raddr, 0)
	}()
}
