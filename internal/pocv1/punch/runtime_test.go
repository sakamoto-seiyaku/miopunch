package punch

import (
	"context"
	"errors"
	"net"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/udpowner"
	legacywire "github.com/miopunch/miopunch/internal/wire"
)

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

	remoteAddr := &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 5009}
	localCandidate := Candidate{Kind: CandidateKindHost, Addr: conn.LocalAddr().String()}
	remoteCandidate := Candidate{Kind: CandidateKindHost, Addr: remoteAddr.String()}
	cfg := LoadedConfig{
		UDPConn:            conn,
		LocalCandidates:    []Candidate{localCandidate},
		AttemptConcurrency: 1,
		AttemptBudget:      time.Second,
		AttemptPair: func(ctx context.Context, demux *udpowner.TraversalDemux, plan pairPlan, key []byte) (AttemptPairResult, error) {
			return AttemptPairResult{RemoteAddr: remoteAddr, Path: PathDirectIPv4}, nil
		},
	}
	remote := trustedRemote{
		PeerID:           mustCanonicalPeerID(t, "AAAAAAAAAAAAAAAAAAAAAAAAAA"),
		MemberCredential: []byte{0x01, 0x02, 0x03},
	}

	got, err := runPunch(
		context.Background(),
		cfg,
		remote,
		mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA"),
		[]byte("0123456789abcdef"),
		[]Candidate{remoteCandidate},
		true,
	)
	if err != nil {
		t.Fatalf("runPunch() error = %v, want nil", err)
	}
	if got.Conn != conn {
		t.Fatalf("runPunch().Conn = %v, want original conn %v", got.Conn, conn)
	}
	if got.RemoteAddr == nil || got.RemoteAddr.String() != remoteAddr.String() {
		t.Fatalf("runPunch().RemoteAddr = %#v, want %q", got.RemoteAddr, remoteAddr.String())
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
	if got.Evidence.SelectedRemoteUDP != remoteAddr.String() {
		t.Fatalf("runPunch().Evidence.SelectedRemoteUDP = %q, want %q", got.Evidence.SelectedRemoteUDP, remoteAddr.String())
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
