package dataplane

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/miopunch/miopunch/event"
)

const (
	maxStreamOpenFrame = 64 * 1024
	// DefaultSessionIdleTimeout is used when a session does not specify an idle timeout.
	DefaultSessionIdleTimeout = 2 * time.Minute
)

// PathFamily identifies the connectivity path family backing a peer session.
type PathFamily string

const (
	PathFamilyUnknown PathFamily = "unknown"
	PathFamilyUDP4    PathFamily = "udp4"
	PathFamilyUDP6    PathFamily = "udp6"
	PathFamilyTCP4    PathFamily = "tcp4"
	PathFamilyTCP6    PathFamily = "tcp6"
)

// CloseReason identifies why a peer session or logical stream closed.
type CloseReason string

const (
	CloseReasonIdleTimeout             CloseReason = "idle_timeout"
	CloseReasonDaemonShutdown          CloseReason = "daemon_shutdown"
	CloseReasonSessionSuperseded       CloseReason = "session_superseded"
	CloseReasonIdentityConfigChange    CloseReason = "identity_config_change"
	CloseReasonAuthorizationRevocation CloseReason = "authorization_revocation"
	CloseReasonStreamProtocolError     CloseReason = "stream_protocol_error"
	CloseReasonTransportFatal          CloseReason = "transport_fatal"
	CloseReasonLogicalStreamComplete   CloseReason = "logical_stream_complete"
)

// StreamKind identifies the logical service carried by a stream.
type StreamKind string

const (
	StreamKindShellV0   StreamKind = "shell.v0"
	StreamKindPayloadV0 StreamKind = "payload.v0"
)

// SessionKey identifies an in-memory peer transport session.
type SessionKey struct {
	RemotePeerID string
	Protocol     Protocol
	SecurityID   string
	PathFamily   PathFamily
}

// PathFamilyFromAttemptPath maps connectivity attempt paths to session path families.
func PathFamilyFromAttemptPath(path string) PathFamily {
	switch strings.TrimSpace(path) {
	case "direct_ipv6":
		return PathFamilyUDP6
	case "direct_ipv4", "punching_ipv4":
		return PathFamilyUDP4
	case "direct_tcp6":
		return PathFamilyTCP6
	case "direct_tcp4", "punching_tcp4":
		return PathFamilyTCP4
	default:
		return PathFamilyUnknown
	}
}

// Normalize returns key with whitespace removed and defaults applied.
func (k SessionKey) Normalize() SessionKey {
	k.RemotePeerID = strings.TrimSpace(k.RemotePeerID)
	k.Protocol = Protocol(strings.TrimSpace(string(k.Protocol)))
	k.SecurityID = strings.TrimSpace(k.SecurityID)
	k.PathFamily = PathFamily(strings.TrimSpace(string(k.PathFamily)))
	if k.PathFamily == "" {
		k.PathFamily = PathFamilyUnknown
	}
	return k
}

// StreamOpen is sent as the first frame on every logical stream.
type StreamOpen struct {
	Kind     StreamKind        `json:"kind"`
	Metadata map[string]string `json:"metadata,omitempty"`
}

// Normalize returns open with whitespace removed and metadata copied.
func (o StreamOpen) Normalize() StreamOpen {
	o.Kind = StreamKind(strings.TrimSpace(string(o.Kind)))
	if len(o.Metadata) == 0 {
		return o
	}

	meta := make(map[string]string, len(o.Metadata))
	for k, v := range o.Metadata {
		k = strings.TrimSpace(k)
		if k == "" {
			continue
		}
		meta[k] = strings.TrimSpace(v)
	}
	o.Metadata = meta
	return o
}

// WriteStreamOpen writes the logical stream-open envelope.
func WriteStreamOpen(w io.Writer, open StreamOpen) error {
	data, err := marshalStreamOpen(open)
	if err != nil {
		return err
	}
	return writeFrame(w, data)
}

// ReadStreamOpen reads the logical stream-open envelope.
func ReadStreamOpen(r io.Reader) (StreamOpen, error) {
	data, err := readFrame(r, maxStreamOpenFrame)
	if err != nil {
		return StreamOpen{}, err
	}
	return unmarshalStreamOpen(data)
}

func marshalStreamOpen(open StreamOpen) ([]byte, error) {
	open = open.Normalize()
	if open.Kind == "" {
		return nil, errors.New("stream kind is required")
	}
	data, err := json.Marshal(open)
	if err != nil {
		return nil, fmt.Errorf("marshal stream open: %w", err)
	}
	if len(data) > maxStreamOpenFrame {
		return nil, fmt.Errorf("stream open frame too large: %d", len(data))
	}
	return data, nil
}

func unmarshalStreamOpen(data []byte) (StreamOpen, error) {
	var open StreamOpen
	if err := json.Unmarshal(data, &open); err != nil {
		return StreamOpen{}, fmt.Errorf("unmarshal stream open: %w", err)
	}
	open = open.Normalize()
	if open.Kind == "" {
		return StreamOpen{}, errors.New("stream kind is required")
	}
	return open, nil
}

// AcceptedStream is a logical stream plus its stream-open envelope.
type AcceptedStream struct {
	Stream io.ReadWriteCloser
	Open   StreamOpen
}

// PeerSession is a live per-peer transport session.
type PeerSession interface {
	Key() SessionKey
	OpenStream(ctx context.Context, open StreamOpen) (io.ReadWriteCloser, error)
	AcceptStream(ctx context.Context) (*AcceptedStream, error)
	Close(reason CloseReason) error
	CloseReason() CloseReason
	Healthy() bool
	LastActivity() time.Time
}

type sessionBase struct {
	mu           sync.Mutex
	key          SessionKey
	em           *event.Emitter
	lastActivity time.Time
	closeReason  CloseReason
	closed       bool
	done         chan struct{}
}

func newSessionBase(key SessionKey, em *event.Emitter) sessionBase {
	return sessionBase{
		key:          key.Normalize(),
		em:           em,
		lastActivity: time.Now(),
		done:         make(chan struct{}),
	}
}

func (b *sessionBase) Key() SessionKey {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.key
}

func (b *sessionBase) CloseReason() CloseReason {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.closeReason
}

func (b *sessionBase) Healthy() bool {
	b.mu.Lock()
	defer b.mu.Unlock()
	return !b.closed
}

func (b *sessionBase) LastActivity() time.Time {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.lastActivity
}

func (b *sessionBase) markActivity() {
	b.mu.Lock()
	b.lastActivity = time.Now()
	b.mu.Unlock()
}

func (b *sessionBase) idleFor() time.Duration {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.closed {
		return 0
	}
	return time.Since(b.lastActivity)
}

func (b *sessionBase) closeBase(reason CloseReason, closeFn func() error) error {
	reason = normalizeCloseReason(reason)

	b.mu.Lock()
	if b.closed {
		b.mu.Unlock()
		return nil
	}
	b.closed = true
	b.closeReason = reason
	key := b.key
	done := b.done
	em := b.em
	b.mu.Unlock()

	close(done)
	err := closeFn()
	emitSessionClose(em, key, reason, err)
	return err
}

func normalizeCloseReason(reason CloseReason) CloseReason {
	reason = CloseReason(strings.TrimSpace(string(reason)))
	if reason == "" {
		return CloseReasonTransportFatal
	}
	return reason
}

func emitSessionClose(em *event.Emitter, key SessionKey, reason CloseReason, err error) {
	if em == nil {
		return
	}
	kvs := sessionEventKVs(key)
	kvs["reason"] = string(reason)
	if err != nil {
		em.Fail(event.StageTransport, err, "transport session closed", kvs)
		return
	}
	em.Emit(event.Event{
		Stage: event.StageTransport,
		Kind:  event.KindInfo,
		Name:  "transport.session_close",
		Msg:   "transport session closed",
		KVs:   kvs,
	})
}

func emitSessionOpen(em *event.Emitter, key SessionKey, impl string) {
	if em == nil {
		return
	}
	kvs := sessionEventKVs(key)
	kvs["impl"] = impl
	em.Emit(event.Event{
		Stage: event.StageTransport,
		Kind:  event.KindStart,
		Name:  "transport.session_open",
		Msg:   string(key.Protocol) + " session open",
		KVs:   kvs,
	})
}

func emitLogicalStream(em *event.Emitter, key SessionKey, open StreamOpen, accepted bool) {
	if em == nil {
		return
	}
	kvs := sessionEventKVs(key)
	kvs["stream_kind"] = string(open.Kind)
	name := "transport.stream_open"
	msg := string(key.Protocol) + " stream open"
	if accepted {
		name = "transport.stream_accept"
		msg = string(key.Protocol) + " stream accepted"
	}
	em.Emit(event.Event{
		Stage: event.StageTransport,
		Kind:  event.KindStart,
		Name:  name,
		Msg:   msg,
		KVs:   kvs,
	})
}

func emitLogicalStreamClose(em *event.Emitter, key SessionKey, open StreamOpen, accepted bool, reason CloseReason, err error) {
	if em == nil {
		return
	}
	kvs := sessionEventKVs(key)
	kvs["stream_kind"] = string(open.Kind)
	kvs["reason"] = string(normalizeCloseReason(reason))
	name := "transport.stream_close"
	msg := string(key.Protocol) + " stream closed"
	if accepted {
		name = "transport.stream_accept_close"
		msg = string(key.Protocol) + " accepted stream closed"
	}
	if err != nil {
		kvs["close_err"] = err.Error()
	}
	em.Emit(event.Event{
		Stage: event.StageTransport,
		Kind:  event.KindInfo,
		Name:  name,
		Msg:   msg,
		KVs:   kvs,
	})
}

func sessionEventKVs(key SessionKey) map[string]any {
	key = key.Normalize()
	return map[string]any{
		"remote_peer_id": key.RemotePeerID,
		"data_proto":     string(key.Protocol),
		"security_id":    key.SecurityID,
		"path_family":    string(key.PathFamily),
	}
}

type logicalStream struct {
	rwc        io.ReadWriteCloser
	onActivity func()
	em         *event.Emitter
	key        SessionKey
	open       StreamOpen
	accept     bool
	once       sync.Once
}

func (s *logicalStream) Read(p []byte) (int, error) {
	n, err := s.rwc.Read(p)
	if n > 0 && s.onActivity != nil {
		s.onActivity()
	}
	return n, err
}

func (s *logicalStream) Write(p []byte) (int, error) {
	n, err := s.rwc.Write(p)
	if n > 0 && s.onActivity != nil {
		s.onActivity()
	}
	return n, err
}

func (s *logicalStream) Close() error {
	var err error
	s.once.Do(func() {
		err = s.rwc.Close()
		if s.onActivity != nil {
			s.onActivity()
		}
		emitLogicalStreamClose(s.em, s.key, s.open, s.accept, CloseReasonLogicalStreamComplete, err)
	})
	return err
}

func (s *logicalStream) SetDeadline(t time.Time) error {
	conn, ok := s.rwc.(interface{ SetDeadline(time.Time) error })
	if !ok {
		return nil
	}
	return conn.SetDeadline(t)
}

func (s *logicalStream) SetReadDeadline(t time.Time) error {
	conn, ok := s.rwc.(interface{ SetReadDeadline(time.Time) error })
	if !ok {
		return nil
	}
	return conn.SetReadDeadline(t)
}

func (s *logicalStream) SetWriteDeadline(t time.Time) error {
	conn, ok := s.rwc.(interface{ SetWriteDeadline(time.Time) error })
	if !ok {
		return nil
	}
	return conn.SetWriteDeadline(t)
}

// SessionManager stores in-memory peer sessions for a daemon/runtime.
type SessionManager struct {
	mu           sync.Mutex
	sessions     map[SessionKey]PeerSession
	recentClosed []SessionSummary
	changeHook   func()
}

// SessionSummary is a stable subset of a live peer session, suitable for diagnostics.
type SessionSummary struct {
	Key                   SessionKey       `json:"key"`
	Healthy               bool             `json:"healthy"`
	LastActivityUnixMilli int64            `json:"last_activity_unix_ms,omitempty"`
	ClosedAtUnixMilli     int64            `json:"closed_at_unix_ms,omitempty"`
	CloseReason           CloseReason      `json:"close_reason,omitempty"`
	PathFacts             SessionPathFacts `json:"path_facts,omitempty"`
}

// SessionPathFacts contains safe selected-path facts for a peer session.
type SessionPathFacts struct {
	LocalEndpoint  string `json:"local_endpoint,omitempty"`
	RemoteEndpoint string `json:"remote_endpoint,omitempty"`
	SelectedPath   string `json:"selected_path,omitempty"`
	PunchStatus    string `json:"punch_status,omitempty"`
	Port           string `json:"port,omitempty"`
}

// SessionPathReporter is implemented by sessions that can report selected-path facts.
type SessionPathReporter interface {
	SessionPathFacts() SessionPathFacts
}

// Normalize returns a trimmed copy of f and derives Port from RemoteEndpoint when possible.
func (f SessionPathFacts) Normalize() SessionPathFacts {
	f.LocalEndpoint = strings.TrimSpace(f.LocalEndpoint)
	f.RemoteEndpoint = strings.TrimSpace(f.RemoteEndpoint)
	f.SelectedPath = strings.TrimSpace(f.SelectedPath)
	f.PunchStatus = strings.TrimSpace(f.PunchStatus)
	f.Port = strings.TrimSpace(f.Port)
	if f.Port == "" && f.RemoteEndpoint != "" {
		_, port, err := net.SplitHostPort(f.RemoteEndpoint)
		if err == nil {
			f.Port = strings.TrimSpace(port)
		}
	}
	return f
}

// Empty reports whether f contains no selected-path facts.
func (f SessionPathFacts) Empty() bool {
	f = f.Normalize()
	return f.LocalEndpoint == "" &&
		f.RemoteEndpoint == "" &&
		f.SelectedPath == "" &&
		f.PunchStatus == "" &&
		f.Port == ""
}

// Merge returns f with non-empty fields from override applied.
func (f SessionPathFacts) Merge(override SessionPathFacts) SessionPathFacts {
	f = f.Normalize()
	override = override.Normalize()
	if override.LocalEndpoint != "" {
		f.LocalEndpoint = override.LocalEndpoint
	}
	if override.RemoteEndpoint != "" {
		f.RemoteEndpoint = override.RemoteEndpoint
	}
	if override.SelectedPath != "" {
		f.SelectedPath = override.SelectedPath
	}
	if override.PunchStatus != "" {
		f.PunchStatus = override.PunchStatus
	}
	if override.Port != "" {
		f.Port = override.Port
	}
	return f.Normalize()
}

// PathFactsFromSession returns selected-path facts reported by sess, if any.
func PathFactsFromSession(sess PeerSession) SessionPathFacts {
	reporter, ok := sess.(SessionPathReporter)
	if !ok || reporter == nil {
		return SessionPathFacts{}
	}
	return reporter.SessionPathFacts().Normalize()
}

// NewSessionManager returns an empty in-memory peer session manager.
func NewSessionManager() *SessionManager {
	return &SessionManager{sessions: make(map[SessionKey]PeerSession)}
}

func (m *SessionManager) SetChangeHook(fn func()) {
	if m == nil {
		return
	}
	m.mu.Lock()
	m.changeHook = fn
	m.mu.Unlock()
}

// Put stores a session and closes any older session with the same key.
func (m *SessionManager) Put(sess PeerSession) {
	if m == nil || sess == nil {
		return
	}
	key := sess.Key().Normalize()
	m.mu.Lock()
	old := m.sessions[key]
	m.sessions[key] = sess
	if old != nil && old != sess {
		m.recordClosedLocked(old, CloseReasonSessionSuperseded)
	}
	changeHook := m.changeHook
	m.mu.Unlock()

	if old != nil && old != sess {
		_ = old.Close(CloseReasonSessionSuperseded)
	}
	if changeHook != nil {
		changeHook()
	}
}

// Get returns a healthy session for key.
func (m *SessionManager) Get(key SessionKey) (PeerSession, bool) {
	if m == nil {
		return nil, false
	}
	key = key.Normalize()
	m.mu.Lock()
	sess := m.sessions[key]
	m.mu.Unlock()
	if sess == nil || !sess.Healthy() {
		return nil, false
	}
	return sess, true
}

// Find returns any healthy session matching non-empty fields in key.
func (m *SessionManager) Find(key SessionKey) (PeerSession, bool) {
	if m == nil {
		return nil, false
	}
	key = key.Normalize()

	m.mu.Lock()
	defer m.mu.Unlock()
	for sessKey, sess := range m.sessions {
		if sess == nil || !sess.Healthy() {
			continue
		}
		if key.RemotePeerID != "" && sessKey.RemotePeerID != key.RemotePeerID {
			continue
		}
		if key.Protocol != "" && sessKey.Protocol != key.Protocol {
			continue
		}
		if key.SecurityID != "" && sessKey.SecurityID != key.SecurityID {
			continue
		}
		if key.PathFamily != "" && key.PathFamily != PathFamilyUnknown && sessKey.PathFamily != key.PathFamily {
			continue
		}
		return sess, true
	}
	return nil, false
}

// Close closes and removes a session.
func (m *SessionManager) Close(key SessionKey, reason CloseReason) {
	if m == nil {
		return
	}
	key = key.Normalize()
	m.mu.Lock()
	sess := m.sessions[key]
	delete(m.sessions, key)
	if sess != nil {
		m.recordClosedLocked(sess, reason)
	}
	changeHook := m.changeHook
	m.mu.Unlock()
	if sess != nil {
		_ = sess.Close(reason)
	}
	if sess != nil && changeHook != nil {
		changeHook()
	}
}

// CloseIfMatch closes and removes sess only if it is still the current session
// for its key.
func (m *SessionManager) CloseIfMatch(sess PeerSession, reason CloseReason) {
	if m == nil || sess == nil {
		return
	}
	key := sess.Key().Normalize()
	m.mu.Lock()
	current := m.sessions[key]
	if current != sess {
		m.mu.Unlock()
		return
	}
	delete(m.sessions, key)
	m.recordClosedLocked(sess, reason)
	changeHook := m.changeHook
	m.mu.Unlock()

	_ = sess.Close(reason)
	if changeHook != nil {
		changeHook()
	}
}

// CloseAll closes and removes every stored session.
func (m *SessionManager) CloseAll(reason CloseReason) {
	if m == nil {
		return
	}
	m.mu.Lock()
	sessions := make([]PeerSession, 0, len(m.sessions))
	for key, sess := range m.sessions {
		delete(m.sessions, key)
		if sess != nil {
			m.recordClosedLocked(sess, reason)
			sessions = append(sessions, sess)
		}
	}
	changeHook := m.changeHook
	m.mu.Unlock()

	for _, sess := range sessions {
		_ = sess.Close(reason)
	}
	if len(sessions) > 0 && changeHook != nil {
		changeHook()
	}
}

const maxRecentClosedSessionSummaries = 32

func (m *SessionManager) recordClosedLocked(sess PeerSession, reason CloseReason) {
	if sess == nil {
		return
	}
	if reason == "" {
		reason = sess.CloseReason()
	}
	summary := SessionSummary{
		Key:                   sess.Key().Normalize(),
		Healthy:               false,
		LastActivityUnixMilli: sess.LastActivity().UTC().UnixMilli(),
		ClosedAtUnixMilli:     time.Now().UTC().UnixMilli(),
		CloseReason:           normalizeCloseReason(reason),
		PathFacts:             PathFactsFromSession(sess),
	}
	m.recentClosed = append(m.recentClosed, summary)
	if len(m.recentClosed) > maxRecentClosedSessionSummaries {
		m.recentClosed = append([]SessionSummary(nil), m.recentClosed[len(m.recentClosed)-maxRecentClosedSessionSummaries:]...)
	}
}

// ListSummaries returns stable summaries of currently stored healthy sessions.
func (m *SessionManager) ListSummaries() []SessionSummary {
	return filterSessionSummaries(m.ListAllSummaries(), true)
}

// ListAllSummaries returns summaries of stored sessions plus recent closures.
func (m *SessionManager) ListAllSummaries() []SessionSummary {
	if m == nil {
		return nil
	}
	m.mu.Lock()
	out := make([]SessionSummary, 0, len(m.sessions)+len(m.recentClosed))
	for k, sess := range m.sessions {
		if sess == nil {
			continue
		}
		healthy := sess.Healthy()
		out = append(out, SessionSummary{
			Key:                   k.Normalize(),
			Healthy:               healthy,
			LastActivityUnixMilli: sess.LastActivity().UTC().UnixMilli(),
			CloseReason:           sess.CloseReason(),
			PathFacts:             PathFactsFromSession(sess),
		})
	}
	out = append(out, m.recentClosed...)
	m.mu.Unlock()

	sort.Slice(out, func(i, j int) bool {
		a, b := out[i].Key, out[j].Key
		if a.RemotePeerID != b.RemotePeerID {
			return a.RemotePeerID < b.RemotePeerID
		}
		if a.Protocol != b.Protocol {
			return string(a.Protocol) < string(b.Protocol)
		}
		if a.SecurityID != b.SecurityID {
			return a.SecurityID < b.SecurityID
		}
		if a.PathFamily != b.PathFamily {
			return string(a.PathFamily) < string(b.PathFamily)
		}
		return out[i].ClosedAtUnixMilli < out[j].ClosedAtUnixMilli
	})
	return out
}

func filterSessionSummaries(in []SessionSummary, healthy bool) []SessionSummary {
	out := make([]SessionSummary, 0, len(in))
	for _, summary := range in {
		if summary.Healthy == healthy {
			out = append(out, summary)
		}
	}
	return out
}
