package controlplane

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
)

var (
	ErrInviteUsesExhausted = errors.New("invite uses exhausted")
)

type inviteRecord struct {
	InviteTopic     string            `json:"invite_topic"`
	ExpiresAtUnixMs int64             `json:"expires_at_unix_ms"`
	MaxUses         int               `json:"max_uses"`
	UsesLeft        int               `json:"uses_left"`
	HandledRequests map[string]string `json:"handled_requests,omitempty"` // request_msg_id -> response_ct_b64url
}

// InviteIDFromTopic derives invite_id for indexing and filenames:
// base32(raw,no-pad, sha256(invite_topic)[:16]).
func InviteIDFromTopic(inviteTopic string) (string, error) {
	trimmed := strings.TrimSpace(inviteTopic)
	if trimmed == "" {
		return "", errors.New("empty invite_topic")
	}
	sum := sha256Sum16(trimmed)
	return base32RawNoPad.EncodeToString(sum[:]), nil
}

func sha256Sum16(s string) [16]byte {
	sum := sha256Sum32([]byte(s))
	var out [16]byte
	copy(out[:], sum[:16])
	return out
}

func sha256Sum32(b []byte) [32]byte {
	return sha256.Sum256(b)
}

// InviteStore provides minimal persistent issuer-side accounting for invites:
// uses_left and handled_requests (request_msg_id -> cached response ciphertext).
//
// It is safe for concurrent use.
type InviteStore struct {
	stateDir string

	locksMu sync.Mutex
	locks   map[string]*sync.Mutex
}

func NewInviteStore(stateDir string) (*InviteStore, error) {
	if strings.TrimSpace(stateDir) == "" {
		return nil, errors.New("empty state_dir")
	}
	return &InviteStore{
		stateDir: stateDir,
		locks:    make(map[string]*sync.Mutex),
	}, nil
}

func (s *InviteStore) invitePath(inviteID string) string {
	return filepath.Join(s.stateDir, "invites", inviteID+".json")
}

func (s *InviteStore) lockForInvite(inviteID string) *sync.Mutex {
	s.locksMu.Lock()
	defer s.locksMu.Unlock()

	if s.locks == nil {
		s.locks = make(map[string]*sync.Mutex)
	}
	if mu, ok := s.locks[inviteID]; ok {
		return mu
	}
	mu := &sync.Mutex{}
	s.locks[inviteID] = mu
	return mu
}

func (s *InviteStore) ensureInvitesDir() (string, error) {
	dir := filepath.Join(s.stateDir, "invites")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", fmt.Errorf("mkdir invites dir: %w", err)
	}
	return dir, nil
}

func (s *InviteStore) loadInvite(inviteID string) (inviteRecord, error) {
	path := s.invitePath(inviteID)
	data, err := os.ReadFile(path)
	if err != nil {
		return inviteRecord{}, err
	}
	var rec inviteRecord
	if err := json.Unmarshal(data, &rec); err != nil {
		return inviteRecord{}, fmt.Errorf("unmarshal invite record: %w", err)
	}
	if rec.HandledRequests == nil {
		rec.HandledRequests = make(map[string]string)
	}
	return rec, nil
}

func (s *InviteStore) loadOrInitInviteLocked(inviteID string, inviteTopic string, expiresAtUnixMs int64, maxUses int) (inviteRecord, error) {
	if _, err := s.ensureInvitesDir(); err != nil {
		return inviteRecord{}, err
	}

	rec, err := s.loadInvite(inviteID)
	if err == nil {
		if strings.TrimSpace(rec.InviteTopic) == "" || rec.InviteTopic != inviteTopic {
			return inviteRecord{}, fmt.Errorf("invite record invite_topic mismatch: have=%q want=%q", rec.InviteTopic, inviteTopic)
		}
		return rec, nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return inviteRecord{}, err
	}

	rec = inviteRecord{
		InviteTopic:     inviteTopic,
		ExpiresAtUnixMs: expiresAtUnixMs,
		MaxUses:         maxUses,
		UsesLeft:        maxUses,
		HandledRequests: make(map[string]string),
	}
	if err := s.writeInvite(inviteID, rec); err != nil {
		return inviteRecord{}, err
	}
	return rec, nil
}

func (s *InviteStore) writeInvite(inviteID string, rec inviteRecord) error {
	data, err := json.Marshal(rec)
	if err != nil {
		return fmt.Errorf("marshal invite record: %w", err)
	}

	path := s.invitePath(inviteID)
	if err := writeFileAtomic(path, data, 0o600); err != nil {
		return fmt.Errorf("write invite record: %w", err)
	}
	return nil
}

// EnsureInvite ensures the invite record exists on disk, initializing it if missing.
func (s *InviteStore) EnsureInvite(inviteTopic string, expiresAtUnixMs int64, maxUses int) (string, error) {
	if s == nil {
		return "", errors.New("nil invite store")
	}
	inviteID, err := InviteIDFromTopic(inviteTopic)
	if err != nil {
		return "", err
	}
	if expiresAtUnixMs <= 0 {
		return "", errors.New("invalid invite expires_at_unix_ms")
	}
	if maxUses <= 0 {
		return "", errors.New("invalid invite max_uses")
	}

	inviteMu := s.lockForInvite(inviteID)
	inviteMu.Lock()
	defer inviteMu.Unlock()

	if _, err := s.loadOrInitInviteLocked(inviteID, inviteTopic, expiresAtUnixMs, maxUses); err != nil {
		return "", err
	}
	return inviteID, nil
}

// HandleRequest applies issuer-side idempotency and uses accounting for a single
// invite-scoped request_msg_id:
// - handled hit: returns cached response ciphertext without decrementing uses
// - miss: decrements uses_left exactly once, builds and persists final response ciphertext
func (s *InviteStore) HandleRequest(inviteTopic string, inviteExpiresAtUnixMs int64, maxUses int, requestMsgID string, buildFinalResponseCiphertext func() ([]byte, error)) ([]byte, bool, error) {
	if s == nil {
		return nil, false, errors.New("nil invite store")
	}
	if strings.TrimSpace(requestMsgID) == "" {
		return nil, false, errors.New("empty request_msg_id")
	}
	if buildFinalResponseCiphertext == nil {
		return nil, false, errors.New("nil response builder")
	}

	inviteID, err := InviteIDFromTopic(inviteTopic)
	if err != nil {
		return nil, false, err
	}
	if inviteExpiresAtUnixMs <= 0 {
		return nil, false, errors.New("invalid invite expires_at_unix_ms")
	}
	if maxUses <= 0 {
		return nil, false, errors.New("invalid invite max_uses")
	}

	inviteMu := s.lockForInvite(inviteID)
	inviteMu.Lock()
	defer inviteMu.Unlock()

	rec, err := s.loadOrInitInviteLocked(inviteID, inviteTopic, inviteExpiresAtUnixMs, maxUses)
	if err != nil {
		return nil, false, err
	}

	if b64, ok := rec.HandledRequests[requestMsgID]; ok {
		ct, err := base64.RawURLEncoding.DecodeString(b64)
		if err != nil {
			return nil, false, fmt.Errorf("decode cached response_ct_b64url: %w", err)
		}
		out := make([]byte, len(ct))
		copy(out, ct)
		return out, true, nil
	}

	if rec.UsesLeft <= 0 {
		return nil, false, fmt.Errorf("%w", ErrInviteUsesExhausted)
	}

	rec.UsesLeft--

	ct, err := buildFinalResponseCiphertext()
	if err != nil {
		rec.UsesLeft++
		return nil, false, err
	}
	ctCopy := make([]byte, len(ct))
	copy(ctCopy, ct)

	if rec.HandledRequests == nil {
		rec.HandledRequests = make(map[string]string)
	}
	rec.HandledRequests[requestMsgID] = base64.RawURLEncoding.EncodeToString(ctCopy)

	if err := s.writeInvite(inviteID, rec); err != nil {
		return nil, false, err
	}

	out := make([]byte, len(ctCopy))
	copy(out, ctCopy)
	return out, false, nil
}
