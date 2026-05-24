package controlplane

import (
	"container/list"
	"crypto/sha256"
	"errors"
	"fmt"
	"sync"
	"time"
)

const (
	defaultHandledRequestsMaxEntries = 1024
	defaultHandledRequestsMinTTL     = 10 * time.Minute
)

var (
	ErrRetryInvariantViolation = errors.New("retry invariant violation")
)

// RequestFingerprint captures the stable (non-time) request identity used to
// enforce retry invariants for the same request_msg_id.
//
// It intentionally excludes:
// - route.hop_limit
// - route.created_at_unix_ms
// - route.expires_at_unix_ms
// - signed.sig_b64
type RequestFingerprint struct {
	DstPeerID    string
	SenderPeerID string
	Kind         string
	InReplyTo    string
	BodySHA256   [32]byte
}

func fingerprintFromMessage(m Message) RequestFingerprint {
	bodySum := sha256.Sum256(m.Signed.Body)
	return RequestFingerprint{
		DstPeerID:    m.Route.DstPeerID,
		SenderPeerID: m.Signed.SenderPeerID,
		Kind:         m.Signed.Kind,
		InReplyTo:    m.Signed.InReplyTo,
		BodySHA256:   bodySum,
	}
}

type handledEntry struct {
	requestMsgID string
	expiresAt    time.Time
	fp           RequestFingerprint
	responseCT   []byte
}

type inflightEntry struct {
	fp        RequestFingerprint
	expiresAt time.Time

	done chan struct{}

	resp []byte
	err  error
}

// HandledRequestsCache caches final response ciphertext for handled RPC requests.
//
// It is safe for concurrent use.
type HandledRequestsCache struct {
	mu sync.Mutex

	maxEntries int
	minTTL     time.Duration
	clock      func() time.Time

	ll    *list.List
	items map[string]*list.Element

	inflight map[string]*inflightEntry
}

func NewHandledRequestsCache(maxEntries int, minTTL time.Duration, clock func() time.Time) *HandledRequestsCache {
	if maxEntries <= 0 {
		maxEntries = defaultHandledRequestsMaxEntries
	}
	if minTTL <= 0 {
		minTTL = defaultHandledRequestsMinTTL
	}
	if clock == nil {
		clock = func() time.Time { return time.Now().UTC() }
	}

	return &HandledRequestsCache{
		maxEntries: maxEntries,
		minTTL:     minTTL,
		clock:      clock,
		ll:         list.New(),
		items:      make(map[string]*list.Element, maxEntries),
		inflight:   make(map[string]*inflightEntry),
	}
}

// Handle ensures a request is handled at most once per request_msg_id and returns
// a cached final response ciphertext for duplicate deliveries.
//
// The caller-provided buildFinalResponseCiphertext MUST be deterministic for
// the same logical request and SHOULD return the full ciphertext bytes that can
// be re-sent verbatim on retries (e.g., group wrapper ciphertext).
func (c *HandledRequestsCache) Handle(req Message, buildFinalResponseCiphertext func() ([]byte, error)) ([]byte, bool, error) {
	if c == nil {
		return nil, false, errors.New("nil handled requests cache")
	}

	requestMsgID := req.Route.MsgID
	if requestMsgID == "" {
		return nil, false, errors.New("empty request_msg_id")
	}
	if buildFinalResponseCiphertext == nil {
		return nil, false, errors.New("nil response builder")
	}

	fp := fingerprintFromMessage(req)

	now := c.clock()
	entryExpiresAt := now.Add(c.minTTL)
	if req.Route.ExpiresAtUnixMs > 0 {
		reqExpiresAt := time.UnixMilli(req.Route.ExpiresAtUnixMs)
		if reqExpiresAt.After(now) {
			if d := reqExpiresAt.Sub(now); d > c.minTTL {
				entryExpiresAt = reqExpiresAt
			}
		}
	}

	c.mu.Lock()
	if ele, ok := c.items[requestMsgID]; ok {
		ent := ele.Value.(handledEntry)
		if now.After(ent.expiresAt) {
			c.ll.Remove(ele)
			delete(c.items, requestMsgID)
		} else {
			if ent.fp != fp {
				c.mu.Unlock()
				return nil, false, fmt.Errorf("%w: request_msg_id=%s", ErrRetryInvariantViolation, requestMsgID)
			}
			out := make([]byte, len(ent.responseCT))
			copy(out, ent.responseCT)
			c.ll.MoveToFront(ele)
			c.mu.Unlock()
			return out, true, nil
		}
	}

	if in, ok := c.inflight[requestMsgID]; ok {
		if in.fp != fp {
			c.mu.Unlock()
			return nil, false, fmt.Errorf("%w: request_msg_id=%s", ErrRetryInvariantViolation, requestMsgID)
		}
		done := in.done
		c.mu.Unlock()
		<-done

		if in.err != nil {
			return nil, false, in.err
		}
		out := make([]byte, len(in.resp))
		copy(out, in.resp)
		return out, true, nil
	}

	in := &inflightEntry{
		fp:        fp,
		expiresAt: entryExpiresAt,
		done:      make(chan struct{}),
	}
	c.inflight[requestMsgID] = in
	c.mu.Unlock()

	resp, err := buildFinalResponseCiphertext()

	c.mu.Lock()
	delete(c.inflight, requestMsgID)
	if err != nil {
		in.err = err
		close(in.done)
		c.mu.Unlock()
		return nil, false, err
	}

	respCopy := make([]byte, len(resp))
	copy(respCopy, resp)

	ent := handledEntry{
		requestMsgID: requestMsgID,
		expiresAt:    in.expiresAt,
		fp:           fp,
		responseCT:   respCopy,
	}
	ele := c.ll.PushFront(ent)
	c.items[requestMsgID] = ele
	for c.ll.Len() > c.maxEntries {
		last := c.ll.Back()
		if last == nil {
			break
		}
		c.ll.Remove(last)
		delete(c.items, last.Value.(handledEntry).requestMsgID)
	}

	in.resp = respCopy
	close(in.done)
	c.mu.Unlock()

	out := make([]byte, len(respCopy))
	copy(out, respCopy)
	return out, false, nil
}
