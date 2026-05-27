package punch

import (
	"errors"
	"testing"
	"time"
)

func TestWrapDiagnosticErrorPreservesEvidence(t *testing.T) {
	diag := Diagnostic{
		DialID:             "dial-1",
		RemotePeerID:       "peer-remote",
		LocalCandidates:    []Candidate{{Kind: CandidateKindHost, Addr: "127.0.0.1:4001"}},
		RemoteCandidates:   []Candidate{{Kind: CandidateKindHost, Addr: "127.0.0.1:5001"}},
		PlannedPairCount:   1,
		AttemptConcurrency: 2,
		AttemptBudget:      5 * time.Second,
		AttemptedPairs: []AttemptEvidence{
			{LocalCandidate: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:4001"}, RemoteCandidate: Candidate{Kind: CandidateKindHost, Addr: "127.0.0.1:5001"}, Result: "timeout", Detail: "deadline exceeded"},
		},
	}

	err := wrapDiagnosticError(diag, errors.New("boom"))
	var got *Error
	if !errors.As(err, &got) {
		t.Fatalf("wrapDiagnosticError() error type = %T, want %T", err, &Error{})
	}
	if got.Diagnostic.DialID != diag.DialID {
		t.Fatalf("wrapDiagnosticError().Diagnostic.DialID = %q, want %q", got.Diagnostic.DialID, diag.DialID)
	}
	if len(got.Diagnostic.Facts()) == 0 {
		t.Fatal("wrapDiagnosticError().Diagnostic.Facts() = empty, want evidence")
	}
}
