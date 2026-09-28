package shelllock

import (
	"errors"
	"sync"
	"time"
)

var ErrInUse = errors.New("lock in use")

type Key struct {
	PeerID  string
	Target  string
	Session string
}

type Manager struct {
	mu sync.Mutex

	ttl time.Duration
	now func() time.Time

	nextToken uint64
	locks     map[Key]lockEntry
}

type lockEntry struct {
	token        uint64
	lastActivity time.Time
}

func New(ttl time.Duration) *Manager {
	return NewWithNow(ttl, time.Now)
}

func NewWithNow(ttl time.Duration, now func() time.Time) *Manager {
	if ttl <= 0 {
		ttl = 60 * time.Second
	}
	if now == nil {
		now = time.Now
	}
	return &Manager{
		ttl:   ttl,
		now:   now,
		locks: make(map[Key]lockEntry),
	}
}

type Handle struct {
	m     *Manager
	key   Key
	token uint64

	releaseOnce sync.Once
}

func (m *Manager) Acquire(key Key) (*Handle, error) {
	if m == nil {
		return nil, errors.New("nil lock manager")
	}
	now := m.now()

	m.mu.Lock()
	defer m.mu.Unlock()

	if entry, ok := m.locks[key]; ok {
		if now.Sub(entry.lastActivity) <= m.ttl {
			return nil, ErrInUse
		}
		delete(m.locks, key)
	}

	m.nextToken++
	token := m.nextToken
	m.locks[key] = lockEntry{token: token, lastActivity: now}
	return &Handle{m: m, key: key, token: token}, nil
}

func (h *Handle) Touch() bool {
	if h == nil || h.m == nil {
		return false
	}
	return h.m.touch(h.key, h.token)
}

func (h *Handle) Release() {
	if h == nil {
		return
	}
	h.releaseOnce.Do(func() {
		if h.m == nil {
			return
		}
		h.m.release(h.key, h.token)
	})
}

func (m *Manager) touch(key Key, token uint64) bool {
	now := m.now()

	m.mu.Lock()
	defer m.mu.Unlock()

	entry, ok := m.locks[key]
	if !ok || entry.token != token {
		return false
	}
	entry.lastActivity = now
	m.locks[key] = entry
	return true
}

func (m *Manager) release(key Key, token uint64) {
	m.mu.Lock()
	defer m.mu.Unlock()

	entry, ok := m.locks[key]
	if !ok || entry.token != token {
		return
	}
	delete(m.locks, key)
}
