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
	"time"
)

var (
	ErrInviteUsesExhausted       = errors.New("invite uses exhausted")
	ErrApprovalRequestNotFound   = errors.New("approval request not found")
	ErrApprovalDecisionConflict  = errors.New("approval decision conflict")
	ErrApprovalDecisionInvalid   = errors.New("invalid approval decision")
	ErrApprovalResponseNotCached = errors.New("approval response not cached")
	ErrApprovalMaterialNotCached = errors.New("approval decision material not cached")
)

type inviteRecord struct {
	InviteTopic      string                           `json:"invite_topic"`
	ExpiresAtUnixMs  int64                            `json:"expires_at_unix_ms"`
	MaxUses          int                              `json:"max_uses"`
	UsesLeft         int                              `json:"uses_left"`
	HandledRequests  map[string]string                `json:"handled_requests,omitempty"` // request_msg_id -> response_ct_b64url
	ApprovalRequests map[string]ApprovalRequestRecord `json:"approval_requests,omitempty"`
}

const (
	ApprovalStatusPending  = "pending"
	ApprovalStatusApproved = "approved"
	ApprovalStatusRejected = "rejected"
	ApprovalStatusExpired  = "expired"

	ApprovalDecisionApprove = "approve"
	ApprovalDecisionReject  = "reject"
)

// ApprovalRequestRecord is the persisted, non-secret review state for an
// invite join request. DecisionMaterial is internal restart material and must
// be redacted from desktop and LocalAPI state.
type ApprovalRequestRecord struct {
	ApproveTaskID string `json:"approve_task_id"`
	InviteID      string `json:"invite_id"`
	RequestMsgID  string `json:"request_msg_id"`
	MemberPeerID  string `json:"member_peer_id"`

	Status   string `json:"status"`
	Decision string `json:"decision,omitempty"`

	CreatedAtUnixMs int64 `json:"created_at_unix_ms"`
	UpdatedAtUnixMs int64 `json:"updated_at_unix_ms,omitempty"`
	ExpiresAtUnixMs int64 `json:"expires_at_unix_ms,omitempty"`

	MemberName   string `json:"member_name,omitempty"`
	PlatformHint string `json:"platform,omitempty"`
	V4Hint       string `json:"v4_hint,omitempty"`
	V6Hint       string `json:"v6_hint,omitempty"`

	ResponseCTB64URL string `json:"response_ct_b64url,omitempty"`

	DecisionMaterial *ApprovalDecisionMaterial `json:"decision_material,omitempty"`
}

// ApprovalDecisionMaterial is private persisted material used to publish an
// approval response after daemon restart.
type ApprovalDecisionMaterial struct {
	InviteBrokers                   []string `json:"invite_brokers,omitempty"`
	ReplyTopic                      string   `json:"reply_topic,omitempty"`
	JoinRequestBodyB64URL           string   `json:"join_request_body_b64url,omitempty"`
	MemberEd25519PubB64             string   `json:"member_ed25519_pub_b64,omitempty"`
	MemberX25519PubB64              string   `json:"member_x25519_pub_b64,omitempty"`
	ValidatedAtUnixMs               int64    `json:"validated_at_unix_ms,omitempty"`
	ValidatedRequestExpiresAtUnixMs int64    `json:"validated_request_expires_at_unix_ms,omitempty"`
	ValidatedRequestSenderID        string   `json:"validated_request_sender_id,omitempty"`
}

// ApprovalRequestLookup contains a persisted approval request with invite
// metadata needed to resolve decisions.
type ApprovalRequestLookup struct {
	InviteTopic     string
	ExpiresAtUnixMs int64
	MaxUses         int
	Request         ApprovalRequestRecord
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

func approvalRequestKey(approveTaskID string, requestMsgID string) string {
	return strings.TrimSpace(approveTaskID) + "/" + strings.TrimSpace(requestMsgID)
}

func cloneApprovalDecisionMaterial(in *ApprovalDecisionMaterial) *ApprovalDecisionMaterial {
	if in == nil {
		return nil
	}
	out := *in
	out.InviteBrokers = append([]string(nil), in.InviteBrokers...)
	return &out
}

func redactApprovalRequestRecord(rec ApprovalRequestRecord) ApprovalRequestRecord {
	rec.ResponseCTB64URL = ""
	rec.DecisionMaterial = nil
	return rec
}

func normalizeApprovalDecision(decision string) (string, error) {
	switch strings.TrimSpace(decision) {
	case ApprovalDecisionApprove:
		return ApprovalDecisionApprove, nil
	case ApprovalDecisionReject:
		return ApprovalDecisionReject, nil
	default:
		return "", ErrApprovalDecisionInvalid
	}
}

func isTerminalApprovalStatus(status string) bool {
	switch strings.TrimSpace(status) {
	case ApprovalStatusApproved, ApprovalStatusRejected, ApprovalStatusExpired:
		return true
	default:
		return false
	}
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
	if rec.ApprovalRequests == nil {
		rec.ApprovalRequests = make(map[string]ApprovalRequestRecord)
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
		InviteTopic:      inviteTopic,
		ExpiresAtUnixMs:  expiresAtUnixMs,
		MaxUses:          maxUses,
		UsesLeft:         maxUses,
		HandledRequests:  make(map[string]string),
		ApprovalRequests: make(map[string]ApprovalRequestRecord),
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

// RecordApprovalRequest persists or updates one pending review request without
// consuming invite uses.
func (s *InviteStore) RecordApprovalRequest(inviteTopic string, expiresAtUnixMs int64, maxUses int, rec ApprovalRequestRecord) (ApprovalRequestRecord, bool, error) {
	if s == nil {
		return ApprovalRequestRecord{}, false, errors.New("nil invite store")
	}
	if strings.TrimSpace(rec.ApproveTaskID) == "" {
		return ApprovalRequestRecord{}, false, errors.New("empty approve_task_id")
	}
	if strings.TrimSpace(rec.RequestMsgID) == "" {
		return ApprovalRequestRecord{}, false, errors.New("empty request_msg_id")
	}
	if strings.TrimSpace(rec.MemberPeerID) == "" {
		return ApprovalRequestRecord{}, false, errors.New("empty member_peer_id")
	}

	inviteID, err := InviteIDFromTopic(inviteTopic)
	if err != nil {
		return ApprovalRequestRecord{}, false, err
	}
	if expiresAtUnixMs <= 0 {
		return ApprovalRequestRecord{}, false, errors.New("invalid invite expires_at_unix_ms")
	}
	if maxUses <= 0 {
		return ApprovalRequestRecord{}, false, errors.New("invalid invite max_uses")
	}

	inviteMu := s.lockForInvite(inviteID)
	inviteMu.Lock()
	defer inviteMu.Unlock()

	stored, err := s.loadOrInitInviteLocked(inviteID, inviteTopic, expiresAtUnixMs, maxUses)
	if err != nil {
		return ApprovalRequestRecord{}, false, err
	}
	if stored.ApprovalRequests == nil {
		stored.ApprovalRequests = make(map[string]ApprovalRequestRecord)
	}

	key := approvalRequestKey(rec.ApproveTaskID, rec.RequestMsgID)
	nowUnixMs := rec.UpdatedAtUnixMs
	if nowUnixMs <= 0 {
		nowUnixMs = rec.CreatedAtUnixMs
	}
	if nowUnixMs <= 0 {
		nowUnixMs = time.Now().UTC().UnixMilli()
	}

	if existing, ok := stored.ApprovalRequests[key]; ok {
		if isTerminalApprovalStatus(existing.Status) {
			return existing, false, nil
		}
		existing.MemberPeerID = strings.TrimSpace(rec.MemberPeerID)
		existing.MemberName = strings.TrimSpace(rec.MemberName)
		existing.PlatformHint = strings.TrimSpace(rec.PlatformHint)
		existing.V4Hint = strings.TrimSpace(rec.V4Hint)
		existing.V6Hint = strings.TrimSpace(rec.V6Hint)
		existing.DecisionMaterial = cloneApprovalDecisionMaterial(rec.DecisionMaterial)
		existing.UpdatedAtUnixMs = nowUnixMs
		if existing.ExpiresAtUnixMs <= 0 {
			existing.ExpiresAtUnixMs = expiresAtUnixMs
		}
		stored.ApprovalRequests[key] = existing
		if err := s.writeInvite(inviteID, stored); err != nil {
			return ApprovalRequestRecord{}, false, err
		}
		return existing, false, nil
	}

	rec.ApproveTaskID = strings.TrimSpace(rec.ApproveTaskID)
	rec.InviteID = inviteID
	rec.RequestMsgID = strings.TrimSpace(rec.RequestMsgID)
	rec.MemberPeerID = strings.TrimSpace(rec.MemberPeerID)
	rec.Status = ApprovalStatusPending
	rec.Decision = ""
	if rec.CreatedAtUnixMs <= 0 {
		rec.CreatedAtUnixMs = nowUnixMs
	}
	rec.UpdatedAtUnixMs = nowUnixMs
	if rec.ExpiresAtUnixMs <= 0 {
		rec.ExpiresAtUnixMs = expiresAtUnixMs
	}
	rec.MemberName = strings.TrimSpace(rec.MemberName)
	rec.PlatformHint = strings.TrimSpace(rec.PlatformHint)
	rec.V4Hint = strings.TrimSpace(rec.V4Hint)
	rec.V6Hint = strings.TrimSpace(rec.V6Hint)
	rec.ResponseCTB64URL = ""
	rec.DecisionMaterial = cloneApprovalDecisionMaterial(rec.DecisionMaterial)

	stored.ApprovalRequests[key] = rec
	if err := s.writeInvite(inviteID, stored); err != nil {
		return ApprovalRequestRecord{}, false, err
	}
	return rec, true, nil
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

// ResolveApprovalDecision records a terminal approval decision and returns the
// ciphertext that should be published to the joiner's reply topic.
func (s *InviteStore) ResolveApprovalDecision(inviteTopic string, expiresAtUnixMs int64, maxUses int, approveTaskID string, requestMsgID string, decision string, buildFinalResponseCiphertext func() ([]byte, error)) ([]byte, bool, ApprovalRequestRecord, error) {
	if s == nil {
		return nil, false, ApprovalRequestRecord{}, errors.New("nil invite store")
	}
	approveTaskID = strings.TrimSpace(approveTaskID)
	requestMsgID = strings.TrimSpace(requestMsgID)
	if approveTaskID == "" {
		return nil, false, ApprovalRequestRecord{}, errors.New("empty approve_task_id")
	}
	if requestMsgID == "" {
		return nil, false, ApprovalRequestRecord{}, errors.New("empty request_msg_id")
	}
	decision, err := normalizeApprovalDecision(decision)
	if err != nil {
		return nil, false, ApprovalRequestRecord{}, err
	}
	if buildFinalResponseCiphertext == nil {
		return nil, false, ApprovalRequestRecord{}, errors.New("nil response builder")
	}

	inviteID, err := InviteIDFromTopic(inviteTopic)
	if err != nil {
		return nil, false, ApprovalRequestRecord{}, err
	}
	if expiresAtUnixMs <= 0 {
		return nil, false, ApprovalRequestRecord{}, errors.New("invalid invite expires_at_unix_ms")
	}
	if maxUses <= 0 {
		return nil, false, ApprovalRequestRecord{}, errors.New("invalid invite max_uses")
	}

	inviteMu := s.lockForInvite(inviteID)
	inviteMu.Lock()
	defer inviteMu.Unlock()

	rec, err := s.loadOrInitInviteLocked(inviteID, inviteTopic, expiresAtUnixMs, maxUses)
	if err != nil {
		return nil, false, ApprovalRequestRecord{}, err
	}
	if rec.ApprovalRequests == nil {
		rec.ApprovalRequests = make(map[string]ApprovalRequestRecord)
	}

	key := approvalRequestKey(approveTaskID, requestMsgID)
	approval, ok := rec.ApprovalRequests[key]
	if !ok {
		return nil, false, ApprovalRequestRecord{}, ErrApprovalRequestNotFound
	}

	if isTerminalApprovalStatus(approval.Status) {
		if approval.Decision != decision {
			return nil, false, approval, ErrApprovalDecisionConflict
		}
		ct, err := base64.RawURLEncoding.DecodeString(approval.ResponseCTB64URL)
		if err != nil || len(ct) == 0 {
			return nil, false, approval, ErrApprovalResponseNotCached
		}
		out := make([]byte, len(ct))
		copy(out, ct)
		return out, true, approval, nil
	}

	nowUnixMs := time.Now().UTC().UnixMilli()
	if approval.ExpiresAtUnixMs > 0 && nowUnixMs > approval.ExpiresAtUnixMs {
		approval.Status = ApprovalStatusExpired
		approval.UpdatedAtUnixMs = nowUnixMs
		rec.ApprovalRequests[key] = approval
		if err := s.writeInvite(inviteID, rec); err != nil {
			return nil, false, ApprovalRequestRecord{}, err
		}
		return nil, false, approval, ErrInviteExpired
	}

	if decision == ApprovalDecisionApprove && rec.UsesLeft <= 0 {
		return nil, false, approval, ErrInviteUsesExhausted
	}
	if decision == ApprovalDecisionApprove {
		rec.UsesLeft--
	}

	ct, err := buildFinalResponseCiphertext()
	if err != nil {
		if decision == ApprovalDecisionApprove {
			rec.UsesLeft++
		}
		return nil, false, ApprovalRequestRecord{}, err
	}
	ctCopy := make([]byte, len(ct))
	copy(ctCopy, ct)

	approval.Decision = decision
	approval.UpdatedAtUnixMs = nowUnixMs
	approval.ResponseCTB64URL = base64.RawURLEncoding.EncodeToString(ctCopy)
	if decision == ApprovalDecisionApprove {
		if rec.HandledRequests == nil {
			rec.HandledRequests = make(map[string]string)
		}
		rec.HandledRequests[requestMsgID] = approval.ResponseCTB64URL
		approval.Status = ApprovalStatusApproved
	} else {
		approval.Status = ApprovalStatusRejected
	}

	rec.ApprovalRequests[key] = approval
	if err := s.writeInvite(inviteID, rec); err != nil {
		return nil, false, ApprovalRequestRecord{}, err
	}

	out := make([]byte, len(ctCopy))
	copy(out, ctCopy)
	return out, false, approval, nil
}

// ExpireApprovalRequestsForTask marks all pending requests for one approve task
// as expired.
func (s *InviteStore) ExpireApprovalRequestsForTask(inviteTopic string, expiresAtUnixMs int64, maxUses int, approveTaskID string) (int, error) {
	if s == nil {
		return 0, errors.New("nil invite store")
	}
	approveTaskID = strings.TrimSpace(approveTaskID)
	if approveTaskID == "" {
		return 0, errors.New("empty approve_task_id")
	}

	inviteID, err := InviteIDFromTopic(inviteTopic)
	if err != nil {
		return 0, err
	}
	if expiresAtUnixMs <= 0 {
		return 0, errors.New("invalid invite expires_at_unix_ms")
	}
	if maxUses <= 0 {
		return 0, errors.New("invalid invite max_uses")
	}

	inviteMu := s.lockForInvite(inviteID)
	inviteMu.Lock()
	defer inviteMu.Unlock()

	rec, err := s.loadOrInitInviteLocked(inviteID, inviteTopic, expiresAtUnixMs, maxUses)
	if err != nil {
		return 0, err
	}

	nowUnixMs := time.Now().UTC().UnixMilli()
	changed := 0
	for key, approval := range rec.ApprovalRequests {
		if approval.ApproveTaskID != approveTaskID || approval.Status != ApprovalStatusPending {
			continue
		}
		approval.Status = ApprovalStatusExpired
		approval.UpdatedAtUnixMs = nowUnixMs
		rec.ApprovalRequests[key] = approval
		changed++
	}
	if changed == 0 {
		return 0, nil
	}
	if err := s.writeInvite(inviteID, rec); err != nil {
		return 0, err
	}
	return changed, nil
}

// ListApprovalRequests returns all persisted approval request records in the
// store.
func (s *InviteStore) ListApprovalRequests() ([]ApprovalRequestRecord, error) {
	if s == nil {
		return nil, errors.New("nil invite store")
	}
	dir := filepath.Join(s.stateDir, "invites")
	entries, err := os.ReadDir(dir)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return []ApprovalRequestRecord{}, nil
		}
		return nil, fmt.Errorf("read invites dir: %w", err)
	}

	out := make([]ApprovalRequestRecord, 0)
	for _, entry := range entries {
		if entry.IsDir() || !strings.HasSuffix(entry.Name(), ".json") {
			continue
		}
		data, err := os.ReadFile(filepath.Join(dir, entry.Name()))
		if err != nil {
			return nil, fmt.Errorf("read invite record: %w", err)
		}
		var rec inviteRecord
		if err := json.Unmarshal(data, &rec); err != nil {
			return nil, fmt.Errorf("unmarshal invite record: %w", err)
		}
		for _, approval := range rec.ApprovalRequests {
			out = append(out, redactApprovalRequestRecord(approval))
		}
	}
	return out, nil
}

// LookupApprovalRequest returns one persisted approval request and the invite
// metadata required to resolve a decision.
func (s *InviteStore) LookupApprovalRequest(approveTaskID string, requestMsgID string) (ApprovalRequestLookup, error) {
	if s == nil {
		return ApprovalRequestLookup{}, errors.New("nil invite store")
	}
	approveTaskID = strings.TrimSpace(approveTaskID)
	requestMsgID = strings.TrimSpace(requestMsgID)
	if approveTaskID == "" {
		return ApprovalRequestLookup{}, errors.New("empty approve_task_id")
	}
	if requestMsgID == "" {
		return ApprovalRequestLookup{}, errors.New("empty request_msg_id")
	}

	dir := filepath.Join(s.stateDir, "invites")
	entries, err := os.ReadDir(dir)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return ApprovalRequestLookup{}, ErrApprovalRequestNotFound
		}
		return ApprovalRequestLookup{}, fmt.Errorf("read invites dir: %w", err)
	}

	key := approvalRequestKey(approveTaskID, requestMsgID)
	for _, entry := range entries {
		if entry.IsDir() || !strings.HasSuffix(entry.Name(), ".json") {
			continue
		}
		rec, err := s.loadInvite(strings.TrimSuffix(entry.Name(), ".json"))
		if err != nil {
			return ApprovalRequestLookup{}, err
		}
		approval, ok := rec.ApprovalRequests[key]
		if !ok {
			continue
		}
		approval.DecisionMaterial = cloneApprovalDecisionMaterial(approval.DecisionMaterial)
		return ApprovalRequestLookup{
			InviteTopic:     rec.InviteTopic,
			ExpiresAtUnixMs: rec.ExpiresAtUnixMs,
			MaxUses:         rec.MaxUses,
			Request:         approval,
		}, nil
	}
	return ApprovalRequestLookup{}, ErrApprovalRequestNotFound
}
