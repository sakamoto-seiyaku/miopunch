package pocstate

import "strings"

const (
	ReachabilityHintDirect  = "direct"
	ReachabilityHintEasy    = "easy"
	ReachabilityHintHard1   = "hard1"
	ReachabilityHintHard2   = "hard2"
	ReachabilityHintUnknown = "unknown"
	ReachabilityHintNone    = "none"
)

func NormalizeV4Hint(hint string) string {
	switch strings.ToLower(strings.TrimSpace(hint)) {
	case ReachabilityHintDirect:
		return ReachabilityHintDirect
	case ReachabilityHintEasy:
		return ReachabilityHintEasy
	case ReachabilityHintHard1:
		return ReachabilityHintHard1
	case ReachabilityHintHard2:
		return ReachabilityHintHard2
	case ReachabilityHintNone:
		return ReachabilityHintNone
	case "", ReachabilityHintUnknown:
		return ReachabilityHintUnknown
	default:
		return ReachabilityHintUnknown
	}
}

func NormalizeV6Hint(hint string) string {
	switch strings.ToLower(strings.TrimSpace(hint)) {
	case ReachabilityHintDirect:
		return ReachabilityHintDirect
	case ReachabilityHintEasy:
		return ReachabilityHintEasy
	case ReachabilityHintHard1:
		return ReachabilityHintHard1
	case ReachabilityHintNone:
		return ReachabilityHintNone
	case "", ReachabilityHintUnknown:
		return ReachabilityHintUnknown
	default:
		return ReachabilityHintUnknown
	}
}

func ReachabilityBucket(v4Hint string, v6Hint string) string {
	v4 := NormalizeV4Hint(v4Hint)
	v6 := NormalizeV6Hint(v6Hint)
	if reachabilityRank(v6) < reachabilityRank(v4) {
		return v6
	}
	return v4
}

func ReachabilityRank(bucket string) int {
	return reachabilityRank(bucket)
}

func reachabilityRank(bucket string) int {
	switch strings.ToLower(strings.TrimSpace(bucket)) {
	case ReachabilityHintDirect:
		return 0
	case ReachabilityHintEasy:
		return 1
	case ReachabilityHintHard1:
		return 2
	case ReachabilityHintHard2:
		return 3
	case ReachabilityHintUnknown:
		return 4
	case ReachabilityHintNone:
		return 5
	default:
		return 4
	}
}
