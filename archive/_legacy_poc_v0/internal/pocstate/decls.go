package pocstate

import (
	"bytes"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
)

const (
	declsFormatV0 = "miopunch.decls.poc.v0"

	DeclKindApproveMember = "approve_member"
	DeclKindRevokeMember  = "revoke_member"
)

type ApproveMemberBodyV0 struct {
	MemberPeerID  string `json:"member_peer_id"`
	MemberName    string `json:"member_name,omitempty"`
	Ed25519PubB64 string `json:"ed25519_pub_b64"`
	X25519PubB64  string `json:"x25519_pub_b64"`
	V4Hint        string `json:"v4_hint,omitempty"`
	V6Hint        string `json:"v6_hint,omitempty"`
	PlatformHint  string `json:"platform,omitempty"`
}

type RevokeMemberBodyV0 struct {
	MemberPeerID string `json:"member_peer_id"`
	Reason       string `json:"reason,omitempty"`
}

type DeclV0 struct {
	MsgID           string          `json:"msg_id"`
	CreatedAtUnixMs int64           `json:"created_at_unix_ms"`
	IssuerPeerID    string          `json:"issuer_peer_id"`
	Kind            string          `json:"kind"`
	Body            json.RawMessage `json:"body"`
	SigB64          string          `json:"sig_b64"`
}

type DeclsFileV0 struct {
	Format       string   `json:"format"`
	DeclsHeadB64 string   `json:"decls_head_b64"`
	Decls        []DeclV0 `json:"decls"`
}

func declsPath(stateDir string) string {
	return filepath.Join(stateDir, "decls", "decls.json")
}

var (
	declsLocksMu sync.Mutex
	declsLocks   = make(map[string]*sync.Mutex)
)

func lockForDeclsPath(path string) *sync.Mutex {
	declsLocksMu.Lock()
	defer declsLocksMu.Unlock()

	if mu, ok := declsLocks[path]; ok {
		return mu
	}
	mu := &sync.Mutex{}
	declsLocks[path] = mu
	return mu
}

// UpdateDecls loads decls.json, applies fn, and writes it back atomically.
//
// It serializes updates within the current process to avoid lost updates from
// concurrent approve/revoke/hello acceptance.
func UpdateDecls(stateDir string, fn func(*DeclsFileV0) error) (DeclsFileV0, error) {
	if strings.TrimSpace(stateDir) == "" {
		return DeclsFileV0{}, errors.New("empty state_dir")
	}
	if fn == nil {
		return DeclsFileV0{}, errors.New("nil decls updater")
	}

	path := declsPath(stateDir)
	mu := lockForDeclsPath(path)
	mu.Lock()
	defer mu.Unlock()

	f, err := loadDeclsFileUnlocked(path)
	if err != nil {
		return DeclsFileV0{}, err
	}

	if err := fn(&f); err != nil {
		return DeclsFileV0{}, err
	}

	if err := saveDeclsFileUnlocked(path, f); err != nil {
		return DeclsFileV0{}, err
	}
	return f, nil
}

func LoadDecls(stateDir string) (DeclsFileV0, error) {
	if strings.TrimSpace(stateDir) == "" {
		return DeclsFileV0{}, errors.New("empty state_dir")
	}

	path := declsPath(stateDir)
	mu := lockForDeclsPath(path)
	mu.Lock()
	defer mu.Unlock()

	return loadDeclsFileUnlocked(path)
}

func EnsureDecls(stateDir string) (DeclsFileV0, error) {
	f, err := LoadDecls(stateDir)
	if err == nil {
		return f, nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return DeclsFileV0{}, err
	}

	path := declsPath(stateDir)
	mu := lockForDeclsPath(path)
	mu.Lock()
	defer mu.Unlock()

	f, err = loadDeclsFileUnlocked(path)
	if err == nil {
		return f, nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return DeclsFileV0{}, err
	}

	f = DeclsFileV0{
		Format: declsFormatV0,
		Decls:  []DeclV0{},
	}
	if err := saveDeclsFileUnlocked(path, f); err != nil {
		return DeclsFileV0{}, err
	}
	return f, nil
}

func RawDeclMessages(decls []DeclV0) []json.RawMessage {
	out := make([]json.RawMessage, 0, len(decls))
	for _, decl := range decls {
		data, err := json.Marshal(decl)
		if err != nil {
			continue
		}
		out = append(out, json.RawMessage(data))
	}
	return out
}

func MergeVerifiedDecls(stateDir string, head GovernanceHeadSnapshotV1, rawDecls []json.RawMessage) (DeclsFileV0, error) {
	if len(rawDecls) == 0 {
		return EnsureDecls(stateDir)
	}

	valid := make([]DeclV0, 0, len(rawDecls))
	for _, raw := range rawDecls {
		var decl DeclV0
		if err := json.Unmarshal(raw, &decl); err != nil {
			continue
		}
		if !isMergeableDeclKind(decl.Kind) {
			continue
		}
		pub, ok, err := head.AdminEd25519Pub(decl.IssuerPeerID)
		if err != nil || !ok {
			continue
		}
		if err := VerifyDeclV0(pub, decl); err != nil {
			continue
		}
		valid = append(valid, decl)
	}
	if len(valid) == 0 {
		return EnsureDecls(stateDir)
	}

	return UpdateDecls(stateDir, func(f *DeclsFileV0) error {
		for _, decl := range valid {
			f.Decls = AddDeclSetUnionV0(f.Decls, decl)
		}
		return nil
	})
}

func isMergeableDeclKind(kind string) bool {
	switch strings.TrimSpace(kind) {
	case DeclKindApproveMember, DeclKindRevokeMember:
		return true
	default:
		return false
	}
}

func loadDeclsFileUnlocked(path string) (DeclsFileV0, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return DeclsFileV0{}, err
	}
	var f DeclsFileV0
	if err := json.Unmarshal(data, &f); err != nil {
		return DeclsFileV0{}, fmt.Errorf("unmarshal decls: %w", err)
	}
	if strings.TrimSpace(f.Format) == "" {
		f.Format = declsFormatV0
	}
	if f.Format != declsFormatV0 {
		return DeclsFileV0{}, fmt.Errorf("unsupported decls format: %q", f.Format)
	}
	if f.Decls == nil {
		f.Decls = []DeclV0{}
	}

	// Recompute head as a consistency check (best-effort).
	head, err := DeclsHeadB64V0(f.Decls)
	if err != nil {
		return DeclsFileV0{}, err
	}
	if strings.TrimSpace(f.DeclsHeadB64) != "" && head != strings.TrimSpace(f.DeclsHeadB64) {
		return DeclsFileV0{}, fmt.Errorf("decls_head_b64 mismatch: have=%q want=%q", f.DeclsHeadB64, head)
	}
	f.DeclsHeadB64 = head
	return f, nil
}

func saveDeclsFileUnlocked(path string, f DeclsFileV0) error {
	f.Format = declsFormatV0
	if f.Decls == nil {
		f.Decls = []DeclV0{}
	}

	// Canonicalize by msg_id for stable file ordering.
	merged := make(map[string]DeclV0, len(f.Decls))
	for _, d := range f.Decls {
		if strings.TrimSpace(d.MsgID) == "" {
			continue
		}
		merged[strings.TrimSpace(d.MsgID)] = d
	}
	f.Decls = make([]DeclV0, 0, len(merged))
	for _, d := range merged {
		f.Decls = append(f.Decls, d)
	}
	sort.Slice(f.Decls, func(i, j int) bool { return f.Decls[i].MsgID < f.Decls[j].MsgID })

	head, err := DeclsHeadB64V0(f.Decls)
	if err != nil {
		return err
	}
	f.DeclsHeadB64 = head

	data, err := json.MarshalIndent(f, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal decls: %w", err)
	}
	data = append(data, '\n')

	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return fmt.Errorf("mkdir decls dir: %w", err)
	}
	if err := writeFileAtomic(path, data, 0o600); err != nil {
		return fmt.Errorf("write decls: %w", err)
	}
	return nil
}

func NewApproveMemberDeclV0(now time.Time, issuer Identity, body ApproveMemberBodyV0) (DeclV0, error) {
	now = now.UTC()

	memberPeerID, err := controlplane.CanonicalizePeerID(body.MemberPeerID)
	if err != nil {
		return DeclV0{}, err
	}
	body.MemberPeerID = memberPeerID
	body.V4Hint = NormalizeV4Hint(body.V4Hint)
	body.V6Hint = NormalizeV6Hint(body.V6Hint)

	issuerPeerID, err := controlplane.CanonicalizePeerID(issuer.PeerID)
	if err != nil {
		return DeclV0{}, err
	}

	bodyJSON, err := json.Marshal(body)
	if err != nil {
		return DeclV0{}, fmt.Errorf("marshal approve body: %w", err)
	}

	msgID, err := controlplane.NewMsgID()
	if err != nil {
		return DeclV0{}, err
	}
	msgID, err = controlplane.CanonicalizeMsgID(msgID)
	if err != nil {
		return DeclV0{}, err
	}

	d := DeclV0{
		MsgID:           msgID,
		CreatedAtUnixMs: now.UnixMilli(),
		IssuerPeerID:    issuerPeerID,
		Kind:            DeclKindApproveMember,
		Body:            bodyJSON,
	}
	if err := SignDeclV0(issuer.Ed25519Priv, &d); err != nil {
		return DeclV0{}, err
	}
	return d, nil
}

func NewRevokeMemberDeclV0(now time.Time, issuer Identity, body RevokeMemberBodyV0) (DeclV0, error) {
	now = now.UTC()

	memberPeerID, err := controlplane.CanonicalizePeerID(body.MemberPeerID)
	if err != nil {
		return DeclV0{}, err
	}
	body.MemberPeerID = memberPeerID

	issuerPeerID, err := controlplane.CanonicalizePeerID(issuer.PeerID)
	if err != nil {
		return DeclV0{}, err
	}

	bodyJSON, err := json.Marshal(body)
	if err != nil {
		return DeclV0{}, fmt.Errorf("marshal revoke body: %w", err)
	}

	msgID, err := controlplane.NewMsgID()
	if err != nil {
		return DeclV0{}, err
	}
	msgID, err = controlplane.CanonicalizeMsgID(msgID)
	if err != nil {
		return DeclV0{}, err
	}

	d := DeclV0{
		MsgID:           msgID,
		CreatedAtUnixMs: now.UnixMilli(),
		IssuerPeerID:    issuerPeerID,
		Kind:            DeclKindRevokeMember,
		Body:            bodyJSON,
	}
	if err := SignDeclV0(issuer.Ed25519Priv, &d); err != nil {
		return DeclV0{}, err
	}
	return d, nil
}

type declTranscriptV0 struct {
	MsgID           string          `json:"msg_id"`
	CreatedAtUnixMs int64           `json:"created_at_unix_ms"`
	IssuerPeerID    string          `json:"issuer_peer_id"`
	Kind            string          `json:"kind"`
	Body            json.RawMessage `json:"body"`
}

func SignDeclV0(priv ed25519.PrivateKey, d *DeclV0) error {
	if d == nil {
		return errors.New("nil decl")
	}
	if len(priv) != ed25519.PrivateKeySize {
		return fmt.Errorf("invalid ed25519 priv length: %d", len(priv))
	}

	canonicalMsgID, err := controlplane.CanonicalizeMsgID(d.MsgID)
	if err != nil {
		return err
	}
	d.MsgID = canonicalMsgID

	canonicalIssuer, err := controlplane.CanonicalizePeerID(d.IssuerPeerID)
	if err != nil {
		return err
	}
	d.IssuerPeerID = canonicalIssuer

	t := declTranscriptV0{
		MsgID:           d.MsgID,
		CreatedAtUnixMs: d.CreatedAtUnixMs,
		IssuerPeerID:    d.IssuerPeerID,
		Kind:            strings.TrimSpace(d.Kind),
		Body:            d.Body,
	}
	transcriptJSON, err := json.Marshal(t)
	if err != nil {
		return fmt.Errorf("marshal decl transcript: %w", err)
	}
	sum := sha256.Sum256(transcriptJSON)
	sig := ed25519.Sign(priv, sum[:])
	d.SigB64 = base64.RawURLEncoding.EncodeToString(sig)
	return nil
}

func VerifyDeclV0(pub ed25519.PublicKey, d DeclV0) error {
	if len(pub) != ed25519.PublicKeySize {
		return fmt.Errorf("invalid ed25519 pub length: %d", len(pub))
	}

	msgID, err := controlplane.CanonicalizeMsgID(d.MsgID)
	if err != nil || msgID != d.MsgID {
		return fmt.Errorf("invalid msg_id: %w", err)
	}
	issuerPeerID, err := controlplane.CanonicalizePeerID(d.IssuerPeerID)
	if err != nil || issuerPeerID != d.IssuerPeerID {
		return fmt.Errorf("invalid issuer_peer_id: %w", err)
	}

	sig, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(d.SigB64))
	if err != nil {
		return fmt.Errorf("decode sig_b64: %w", err)
	}
	if len(sig) != ed25519.SignatureSize {
		return fmt.Errorf("invalid signature length: %d", len(sig))
	}

	t := declTranscriptV0{
		MsgID:           d.MsgID,
		CreatedAtUnixMs: d.CreatedAtUnixMs,
		IssuerPeerID:    d.IssuerPeerID,
		Kind:            strings.TrimSpace(d.Kind),
		Body:            d.Body,
	}
	transcriptJSON, err := json.Marshal(t)
	if err != nil {
		return fmt.Errorf("marshal decl transcript: %w", err)
	}
	sum := sha256.Sum256(transcriptJSON)
	if !ed25519.Verify(pub, sum[:], sig) {
		return errors.New("invalid signature")
	}
	return nil
}

func DeclsHeadB64V0(decls []DeclV0) (string, error) {
	hashes := make([][32]byte, 0, len(decls))
	for _, d := range decls {
		// canonical_json(x) = json.Marshal(x) (struct field order, no maps).
		data, err := json.Marshal(d)
		if err != nil {
			return "", fmt.Errorf("marshal decl: %w", err)
		}
		hashes = append(hashes, sha256.Sum256(data))
	}

	sort.Slice(hashes, func(i, j int) bool {
		return bytes.Compare(hashes[i][:], hashes[j][:]) < 0
	})

	buf := make([]byte, 0, 32*len(hashes))
	for _, h := range hashes {
		buf = append(buf, h[:]...)
	}
	sum := sha256.Sum256(buf)
	return base64.RawURLEncoding.EncodeToString(sum[:]), nil
}

func AddDeclSetUnionV0(existing []DeclV0, d DeclV0) []DeclV0 {
	msgID := strings.TrimSpace(d.MsgID)
	if msgID == "" {
		return existing
	}

	out := make([]DeclV0, 0, len(existing)+1)
	found := false
	for _, e := range existing {
		if strings.TrimSpace(e.MsgID) == msgID {
			found = true
		}
		out = append(out, e)
	}
	if !found {
		out = append(out, d)
	}
	return out
}
