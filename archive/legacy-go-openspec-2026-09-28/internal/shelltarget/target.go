package shelltarget

import (
	"fmt"
	"sort"
	"strings"
)

type TargetNotFoundError struct {
	Input   string
	Targets []string
}

func (e TargetNotFoundError) Error() string {
	if strings.TrimSpace(e.Input) == "" {
		return "missing target"
	}
	return fmt.Sprintf("target not found: %q", e.Input)
}

type TargetAmbiguousError struct {
	Input   string
	Matches []string
}

func (e TargetAmbiguousError) Error() string {
	if strings.TrimSpace(e.Input) == "" {
		return "target required"
	}
	return fmt.Sprintf("target ambiguous: %q", e.Input)
}

// Resolve picks a single target from the available list.
//
// Rules (POC v0):
// - Empty input selects the only available target, otherwise errors.
// - Exact match wins.
// - Otherwise, a unique prefix match wins.
func Resolve(input string, targets []string) (string, error) {
	in := strings.TrimSpace(input)

	uniq := make([]string, 0, len(targets))
	seen := map[string]struct{}{}
	for _, t := range targets {
		t = strings.TrimSpace(t)
		if t == "" {
			continue
		}
		if _, ok := seen[t]; ok {
			continue
		}
		seen[t] = struct{}{}
		uniq = append(uniq, t)
	}
	sort.Strings(uniq)

	if in == "" {
		if len(uniq) == 1 {
			return uniq[0], nil
		}
		return "", TargetAmbiguousError{Input: in, Matches: append([]string{}, uniq...)}
	}

	for _, t := range uniq {
		if t == in {
			return t, nil
		}
	}

	matches := make([]string, 0, len(uniq))
	for _, t := range uniq {
		if strings.HasPrefix(t, in) {
			matches = append(matches, t)
		}
	}
	if len(matches) == 0 {
		return "", TargetNotFoundError{Input: in, Targets: append([]string{}, uniq...)}
	}
	if len(matches) > 1 {
		return "", TargetAmbiguousError{Input: in, Matches: matches}
	}
	return matches[0], nil
}
