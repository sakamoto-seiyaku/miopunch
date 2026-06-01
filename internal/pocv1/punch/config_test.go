package punch

import (
	"context"
	"errors"
	"net"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/udpowner"
)

func TestNormalizeCandidatesSortsTrimsAndDedupes(t *testing.T) {
	got, err := normalizeCandidates([]Candidate{
		{Kind: CandidateKindSrflx, Addr: " 203.0.113.10:5000 "},
		{Kind: CandidateKindHost, Addr: "127.0.0.1:4000"},
		{Kind: CandidateKindHost, Addr: "127.0.0.1:4000"},
	})
	if err != nil {
		t.Fatalf("normalizeCandidates() error = %v, want nil", err)
	}
	want := []Candidate{
		{Kind: CandidateKindHost, Addr: "127.0.0.1:4000"},
		{Kind: CandidateKindSrflx, Addr: "203.0.113.10:5000"},
	}
	if len(got) != len(want) {
		t.Fatalf("normalizeCandidates() len = %d, want %d", len(got), len(want))
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("normalizeCandidates()[%d] = %#v, want %#v", i, got[i], want[i])
		}
	}
}

func TestNormalizeCandidatesRejectsInvalidInput(t *testing.T) {
	tests := []struct {
		name       string
		candidates []Candidate
	}{
		{
			name:       "empty addr",
			candidates: []Candidate{{Kind: CandidateKindHost, Addr: " "}},
		},
		{
			name:       "invalid addr",
			candidates: []Candidate{{Kind: CandidateKindHost, Addr: "bad"}},
		},
		{
			name:       "invalid kind",
			candidates: []Candidate{{Kind: CandidateKind("relay"), Addr: "127.0.0.1:4000"}},
		},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			if _, err := normalizeCandidates(tt.candidates); err == nil {
				t.Fatalf("normalizeCandidates(%#v) error = nil, want non-nil", tt.candidates)
			}
		})
	}
}

func TestBuildPairPlansReturnsCartesianProduct(t *testing.T) {
	conn := mustListenUDP(t)
	t.Cleanup(func() { _ = conn.Close() })

	local := []Candidate{
		{Kind: CandidateKindHost, Addr: conn.LocalAddr().String()},
		{Kind: CandidateKindSrflx, Addr: "203.0.113.1:5000"},
	}
	remote := []Candidate{
		{Kind: CandidateKindHost, Addr: "198.51.100.1:6000"},
		{Kind: CandidateKindSrflx, Addr: "198.51.100.2:6001"},
	}
	got, err := buildPairPlans(conn, local, remote, mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA"), true)
	if err != nil {
		t.Fatalf("buildPairPlans() error = %v, want nil", err)
	}
	if len(got) != 4 {
		t.Fatalf("buildPairPlans() len = %d, want %d", len(got), 4)
	}
	if got[0].local != local[0] || got[0].remote != remote[0] {
		t.Fatalf("buildPairPlans()[0] = %#v, want local=%#v remote=%#v", got[0], local[0], remote[0])
	}
}

func TestBuildPairPlansRejectsInvalidRemoteCandidate(t *testing.T) {
	conn := mustListenUDP(t)
	t.Cleanup(func() { _ = conn.Close() })

	_, err := buildPairPlans(conn, []Candidate{{Kind: CandidateKindHost, Addr: conn.LocalAddr().String()}}, []Candidate{{Kind: CandidateKindHost, Addr: "bad"}}, mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA"), true)
	if !errors.Is(err, ErrInvalidOffer) {
		t.Fatalf("buildPairPlans(invalid remote) error = %v, want %v", err, ErrInvalidOffer)
	}
}

func TestBuildPairPlansRejectsNoCandidatePairs(t *testing.T) {
	conn := mustListenUDP(t)
	t.Cleanup(func() { _ = conn.Close() })

	_, err := buildPairPlans(conn, []Candidate{{Kind: CandidateKindHost, Addr: conn.LocalAddr().String()}}, nil, mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA"), true)
	if !errors.Is(err, ErrInvalidOffer) {
		t.Fatalf("buildPairPlans(no candidate pairs) error = %v, want %v", err, ErrInvalidOffer)
	}
}

func TestWithAttemptBudgetReturnsBudgetExceededCause(t *testing.T) {
	ctx, cancel := withAttemptBudget(context.Background(), 5*time.Millisecond)
	defer cancel()

	<-ctx.Done()
	if !errors.Is(context.Cause(ctx), ErrAttemptBudgetExceeded) {
		t.Fatalf("withAttemptBudget() cause = %v, want %v", context.Cause(ctx), ErrAttemptBudgetExceeded)
	}
}

func TestWithAttemptBudgetCancelReturnsContextCanceled(t *testing.T) {
	ctx, cancel := withAttemptBudget(context.Background(), time.Second)
	cancel()

	<-ctx.Done()
	if !errors.Is(context.Cause(ctx), context.Canceled) {
		t.Fatalf("withAttemptBudget(cancel) cause = %v, want %v", context.Cause(ctx), context.Canceled)
	}
}

func TestExecutePairPlansMarksGeneralFailure(t *testing.T) {
	cfg := LoadedConfig{
		AttemptConcurrency: 1,
		AttemptPair: func(ctx context.Context, demux *udpowner.TraversalDemux, plan pairPlan, key []byte) (AttemptPairResult, error) {
			return AttemptPairResult{}, errors.New("boom")
		},
	}
	plans := []pairPlan{
		{index: 0, local: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:1"}, remote: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:2"}},
	}
	_, evidence, err := executePairPlans(context.Background(), cfg, nil, plans, []byte("0123456789abcdef"))
	if err == nil {
		t.Fatalf("executePairPlans(general failure) error = nil, want non-nil")
	}
	if evidence[0].Result != "failed" {
		t.Fatalf("executePairPlans(general failure) evidence[0].Result = %q, want %q", evidence[0].Result, "failed")
	}
}

func TestExecutePairPlansCancelsLosersAfterWinner(t *testing.T) {
	started := make(chan struct{}, 1)
	allowWinner := make(chan struct{})
	loserDone := make(chan struct{}, 1)
	cfg := LoadedConfig{
		AttemptConcurrency: 2,
		AttemptPair: func(ctx context.Context, demux *udpowner.TraversalDemux, plan pairPlan, key []byte) (AttemptPairResult, error) {
			if plan.index == 1 {
				started <- struct{}{}
				<-ctx.Done()
				loserDone <- struct{}{}
				return AttemptPairResult{}, ctx.Err()
			}
			<-allowWinner
			if plan.index == 0 {
				return AttemptPairResult{RemoteAddr: &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 5002}, Path: PathPunchingIPv4}, nil
			}
			return AttemptPairResult{}, errors.New("unexpected attempt index")
		},
	}
	plans := []pairPlan{
		{index: 0, local: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:1"}, remote: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:2"}},
		{index: 1, local: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:1"}, remote: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:3"}},
	}
	done := make(chan struct{})
	var (
		evidence []AttemptEvidence
		err      error
	)
	go func() {
		_, evidence, err = executePairPlans(context.Background(), cfg, nil, plans, []byte("0123456789abcdef"))
		close(done)
	}()
	select {
	case <-started:
	case <-time.After(time.Second):
		t.Fatalf("executePairPlans(winner cancel) loser did not start")
	}
	close(allowWinner)
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatalf("executePairPlans(winner cancel) did not finish")
	}
	if err != nil {
		t.Fatalf("executePairPlans(winner cancel) error = %v, want nil", err)
	}
	select {
	case <-loserDone:
	case <-time.After(time.Second):
		t.Fatalf("executePairPlans(winner cancel) loser did not observe cancellation")
	}
	if evidence[0].Result != "selected" {
		t.Fatalf("executePairPlans(winner cancel) evidence[0].Result = %q, want %q", evidence[0].Result, "selected")
	}
	if evidence[1].Result != "canceled" {
		t.Fatalf("executePairPlans(winner cancel) evidence[1].Result = %q, want %q", evidence[1].Result, "canceled")
	}
}

func TestPathResultCloseWithNilConnIsNoop(t *testing.T) {
	if err := (PathResult{}).Close(); err != nil {
		t.Fatalf("PathResult{}.Close() error = %v, want nil", err)
	}
}
