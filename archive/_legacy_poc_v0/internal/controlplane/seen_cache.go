package controlplane

import (
	"container/list"
	"sync"
	"time"
)

const (
	defaultSeenMaxEntries = 8192
	defaultSeenTTL        = 10 * time.Minute
)

type SeenCache struct {
	mu sync.Mutex

	maxEntries int
	ttl        time.Duration
	clock      func() time.Time

	ll    *list.List
	items map[string]*list.Element
}

type seenEntry struct {
	msgID     string
	expiresAt time.Time
}

func NewSeenCache(maxEntries int, ttl time.Duration, clock func() time.Time) *SeenCache {
	if maxEntries <= 0 {
		maxEntries = defaultSeenMaxEntries
	}
	if ttl <= 0 {
		ttl = defaultSeenTTL
	}
	if clock == nil {
		clock = func() time.Time { return time.Now().UTC() }
	}

	return &SeenCache{
		maxEntries: maxEntries,
		ttl:        ttl,
		clock:      clock,
		ll:         list.New(),
		items:      make(map[string]*list.Element, maxEntries),
	}
}

func (c *SeenCache) SeenBefore(msgID string) bool {
	if c == nil {
		return false
	}
	if msgID == "" {
		return false
	}

	now := c.clock()

	c.mu.Lock()
	defer c.mu.Unlock()

	if ele, ok := c.items[msgID]; ok {
		ent := ele.Value.(seenEntry)
		if now.After(ent.expiresAt) {
			c.ll.Remove(ele)
			delete(c.items, msgID)
		} else {
			ent.expiresAt = now.Add(c.ttl)
			ele.Value = ent
			c.ll.MoveToFront(ele)
			return true
		}
	}

	ent := seenEntry{
		msgID:     msgID,
		expiresAt: now.Add(c.ttl),
	}
	ele := c.ll.PushFront(ent)
	c.items[msgID] = ele

	if c.ll.Len() > c.maxEntries {
		last := c.ll.Back()
		if last != nil {
			c.ll.Remove(last)
			delete(c.items, last.Value.(seenEntry).msgID)
		}
	}

	return false
}
