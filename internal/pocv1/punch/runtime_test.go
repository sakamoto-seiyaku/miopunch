package punch

import (
	"context"
	"errors"
	"net"
	"sync/atomic"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/udpowner"
)

func TestExecutePairPlansHonorsConcurrencyAndSelectsWinner(t *testing.T) {
	var inFlight atomic.Int64
	var maxInFlight atomic.Int64

	cfg := LoadedConfig{
		AttemptConcurrency: 2,
		AttemptPair: func(ctx context.Context, demux *udpowner.TraversalDemux, plan pairPlan, key []byte) (*net.UDPAddr, error) {
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
				return &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 5002}, nil
			}
			return nil, context.DeadlineExceeded
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
}

func TestExecutePairPlansAggregatesTimeoutWhenNoWinner(t *testing.T) {
	cfg := LoadedConfig{
		AttemptConcurrency: 1,
		AttemptPair: func(ctx context.Context, demux *udpowner.TraversalDemux, plan pairPlan, key []byte) (*net.UDPAddr, error) {
			return nil, context.DeadlineExceeded
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
		AttemptPair: func(ctx context.Context, demux *udpowner.TraversalDemux, plan pairPlan, key []byte) (*net.UDPAddr, error) {
			<-ctx.Done()
			return nil, ctx.Err()
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
		AttemptPair: func(ctx context.Context, demux *udpowner.TraversalDemux, plan pairPlan, key []byte) (*net.UDPAddr, error) {
			return remoteAddr, nil
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
	if len(got.Evidence.AttemptedPairs) != 1 {
		t.Fatalf("runPunch().Evidence.AttemptedPairs length = %d, want 1", len(got.Evidence.AttemptedPairs))
	}
	if got.Evidence.AttemptedPairs[0].Result != "selected" {
		t.Fatalf("runPunch().Evidence.AttemptedPairs[0].Result = %q, want %q", got.Evidence.AttemptedPairs[0].Result, "selected")
	}
}
