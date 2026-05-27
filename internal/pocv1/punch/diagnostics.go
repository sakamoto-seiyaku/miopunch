package punch

import (
	"fmt"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
)

// Diagnostic carries structured punch debugging context for real-environment
// investigation and CLI/runtime evidence export.
type Diagnostic struct {
	DialID             string
	RemotePeerID       string
	LocalCandidates    []Candidate
	RemoteCandidates   []Candidate
	PlannedPairCount   int
	AttemptConcurrency int
	AttemptBudget      time.Duration
	AttemptedPairs     []AttemptEvidence
}

// Error wraps a punch failure with structured diagnostic evidence.
type Error struct {
	Diagnostic Diagnostic
	Err        error
}

// Error reports the wrapped punch failure.
func (e *Error) Error() string {
	if e == nil {
		return "<nil>"
	}
	if e.Err == nil {
		return "punch failed"
	}
	return "punch failed: " + e.Err.Error()
}

// Unwrap returns the underlying punch failure.
func (e *Error) Unwrap() error {
	if e == nil {
		return nil
	}
	return e.Err
}

func wrapDiagnosticError(diag Diagnostic, err error) error {
	if err == nil {
		return nil
	}
	return &Error{
		Diagnostic: cloneDiagnostic(diag),
		Err:        err,
	}
}

func cloneDiagnostic(diag Diagnostic) Diagnostic {
	diag.LocalCandidates = append([]Candidate(nil), diag.LocalCandidates...)
	diag.RemoteCandidates = append([]Candidate(nil), diag.RemoteCandidates...)
	diag.AttemptedPairs = append([]AttemptEvidence(nil), diag.AttemptedPairs...)
	return diag
}

// Facts renders bounded, human-readable punch diagnostics as structured facts.
func (d Diagnostic) Facts() []poc.Fact {
	facts := []poc.Fact{
		{Message: "dial_id=" + strings.TrimSpace(d.DialID)},
		{Message: "remote_peer_id=" + strings.TrimSpace(d.RemotePeerID)},
		{Message: fmt.Sprintf("planned_pair_count=%d", d.PlannedPairCount)},
		{Message: fmt.Sprintf("attempt_concurrency=%d", d.AttemptConcurrency)},
		{Message: fmt.Sprintf("attempt_budget_ms=%d", d.AttemptBudget.Milliseconds())},
		{Message: "local_candidates=" + formatCandidates(d.LocalCandidates)},
		{Message: "remote_candidates=" + formatCandidates(d.RemoteCandidates)},
		{Message: "attempt_results=" + summarizeAttemptResults(d.AttemptedPairs)},
	}
	return facts
}

func formatCandidates(candidates []Candidate) string {
	if len(candidates) == 0 {
		return "-"
	}
	parts := make([]string, 0, len(candidates))
	for _, candidate := range candidates {
		parts = append(parts, fmt.Sprintf("%s@%s", candidate.Kind, candidate.Addr))
	}
	return strings.Join(parts, ",")
}

func summarizeAttemptResults(attempts []AttemptEvidence) string {
	if len(attempts) == 0 {
		return "-"
	}
	counts := map[string]int{}
	order := make([]string, 0, len(attempts))
	for _, attempt := range attempts {
		result := strings.TrimSpace(attempt.Result)
		if result == "" {
			result = "unknown"
		}
		if _, ok := counts[result]; !ok {
			order = append(order, result)
		}
		counts[result]++
	}
	parts := make([]string, 0, len(order))
	for _, result := range order {
		parts = append(parts, fmt.Sprintf("%s=%d", result, counts[result]))
	}
	return strings.Join(parts, ",")
}
