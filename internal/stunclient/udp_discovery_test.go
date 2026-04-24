package stunclient

import (
	"context"
	"sync"
	"testing"
	"time"
)

func TestDiscoverWithStrategyStopsAfterEnoughObservation(t *testing.T) {
	t.Parallel()

	var (
		mu      sync.Mutex
		started []string
	)

	discover := func(ctx context.Context, server string) ([]string, time.Duration, error) {
		mu.Lock()
		started = append(started, server)
		mu.Unlock()

		switch server {
		case "s1":
			time.Sleep(10 * time.Millisecond)
			return []string{"198.51.100.1:40001"}, 10 * time.Millisecond, nil
		case "s2":
			time.Sleep(20 * time.Millisecond)
			return []string{"198.51.100.2:40002"}, 20 * time.Millisecond, nil
		default:
			<-ctx.Done()
			return nil, 0, ctx.Err()
		}
	}

	stopFn := func(res DiscoveryResult) bool {
		valid, _ := SanitizeMappedAddrs(res.MappedAddrs)
		return len(valid) >= 2 && res.OkCount >= 2
	}

	res := discoverWithStrategy(context.Background(), []string{"s1", "s2", "s3", "s4", "s5"}, 2, stopFn, discover)
	if res.OkCount != 2 {
		t.Fatalf("OkCount = %d, want 2", res.OkCount)
	}
	if len(res.MappedAddrs) != 2 {
		t.Fatalf("MappedAddrs = %v, want 2 entries", res.MappedAddrs)
	}
	for _, errText := range res.Errors {
		if errText == "s3: context canceled" || errText == "s4: context canceled" || errText == "s5: context canceled" {
			t.Fatalf("unexpected canceled error after stop: %v", res.Errors)
		}
	}

	mu.Lock()
	defer mu.Unlock()
	if len(started) > 3 {
		t.Fatalf("started = %v, want at most initial batch plus one follow-up", started)
	}
	for _, server := range []string{"s4", "s5"} {
		for _, got := range started {
			if got == server {
				t.Fatalf("%s should not have started, got %v", server, started)
			}
		}
	}
}
