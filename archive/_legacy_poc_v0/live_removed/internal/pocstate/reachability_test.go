package pocstate

import "testing"

func TestReachabilityHintNormalizationDropsEndpointLikeValues(t *testing.T) {
	if got := NormalizeV4Hint("1.2.3.4:5000"); got != ReachabilityHintUnknown {
		t.Fatalf("NormalizeV4Hint(endpoint) = %q, want %q", got, ReachabilityHintUnknown)
	}
	if got := NormalizeV6Hint("hard2"); got != ReachabilityHintUnknown {
		t.Fatalf("NormalizeV6Hint(hard2) = %q, want %q", got, ReachabilityHintUnknown)
	}
}

func TestReachabilityBucketChoosesBestFamily(t *testing.T) {
	if got := ReachabilityBucket("hard1", "direct"); got != ReachabilityHintDirect {
		t.Fatalf("ReachabilityBucket(hard1, direct) = %q, want direct", got)
	}
	if got := ReachabilityBucket("easy", "none"); got != ReachabilityHintEasy {
		t.Fatalf("ReachabilityBucket(easy, none) = %q, want easy", got)
	}
}
