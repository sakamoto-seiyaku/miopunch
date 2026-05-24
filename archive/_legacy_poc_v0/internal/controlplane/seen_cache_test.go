package controlplane

import (
	"fmt"
	"sort"
	"testing"
	"time"
)

func dumpSeenCache(c *SeenCache) string {
	if c == nil {
		return "<nil>"
	}

	c.mu.Lock()
	defer c.mu.Unlock()

	var order []string
	for ele := c.ll.Front(); ele != nil; ele = ele.Next() {
		order = append(order, ele.Value.(seenEntry).msgID)
	}

	var keys []string
	for k := range c.items {
		keys = append(keys, k)
	}
	sort.Strings(keys)

	return fmt.Sprintf("order=%v keys=%v len=%d", order, keys, c.ll.Len())
}

func TestSeenCache_TTL(t *testing.T) {
	now := time.Unix(0, 0).UTC()
	clock := func() time.Time { return now }

	c := NewSeenCache(2, 1*time.Second, clock)

	if got := c.SeenBefore("A"); got {
		t.Fatalf("SeenBefore(A) = %t, want %t", got, false)
	}
	if got := c.SeenBefore("A"); !got {
		t.Fatalf("SeenBefore(A) second = %t, want %t", got, true)
	}

	now = now.Add(2 * time.Second)
	if got := c.SeenBefore("A"); got {
		t.Fatalf("SeenBefore(A) after ttl = %t, want %t", got, false)
	}
}

func TestSeenCache_LRUEviction(t *testing.T) {
	now := time.Unix(0, 0).UTC()
	clock := func() time.Time { return now }

	c := NewSeenCache(2, 1*time.Hour, clock)

	_ = c.SeenBefore("A") // insert
	_ = c.SeenBefore("B") // insert
	if got := c.SeenBefore("A"); !got {
		t.Fatalf("SeenBefore(A) = %t, want %t (%s)", got, true, dumpSeenCache(c))
	}

	_ = c.SeenBefore("C") // evicts B

	c.mu.Lock()
	_, hasB := c.items["B"]
	_, hasA := c.items["A"]
	c.mu.Unlock()

	if hasB {
		t.Fatalf("B is unexpectedly still present after eviction (%s)", dumpSeenCache(c))
	}
	if !hasA {
		t.Fatalf("A is unexpectedly missing after eviction (%s)", dumpSeenCache(c))
	}
}
