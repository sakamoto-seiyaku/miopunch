package pocstate

import (
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/miopunch/miopunch/internal/controlplane"
)

const (
	governanceHeadSnapshotFormatV1 = "miopunch.governance.head_snapshot.poc.v1"
)

type GovernanceSignatureV1 struct {
	KeyID  string `json:"key_id"`
	SigB64 string `json:"sig_b64"`
}

type GovernanceSnapshotBodyV1 struct {
	NetID       string `json:"net_id"`
	PrevHashB64 string `json:"prev_hash_b64"`
	Height      int    `json:"height"`

	Owners []string `json:"owners"`
	Admins []string `json:"admins"`
}

func (b GovernanceSnapshotBodyV1) canonicalize() (GovernanceSnapshotBodyV1, []ed25519.PublicKey, []ed25519.PublicKey, error) {
	netID := strings.TrimSpace(b.NetID)
	if netID == "" {
		return GovernanceSnapshotBodyV1{}, nil, nil, errors.New("missing snapshot_body net_id")
	}
	if err := validateNetID(netID); err != nil {
		return GovernanceSnapshotBodyV1{}, nil, nil, err
	}

	prevHashB64 := strings.TrimSpace(b.PrevHashB64)
	height := b.Height
	if height < 0 {
		return GovernanceSnapshotBodyV1{}, nil, nil, fmt.Errorf("invalid snapshot_body height: %d", height)
	}
	if height == 0 {
		if prevHashB64 != "" {
			return GovernanceSnapshotBodyV1{}, nil, nil, errors.New("genesis snapshot_body must have empty prev_hash_b64")
		}
	} else {
		if prevHashB64 == "" {
			return GovernanceSnapshotBodyV1{}, nil, nil, errors.New("non-genesis snapshot_body must have prev_hash_b64")
		}
		prevSum, err := base64.RawURLEncoding.DecodeString(prevHashB64)
		if err != nil || len(prevSum) != sha256.Size {
			return GovernanceSnapshotBodyV1{}, nil, nil, errors.New("invalid snapshot_body prev_hash_b64")
		}
	}

	ownersB64, ownerPubs, err := canonicalizeEd25519PubB64Set("owner", b.Owners)
	if err != nil {
		return GovernanceSnapshotBodyV1{}, nil, nil, err
	}
	if len(ownerPubs) == 0 {
		return GovernanceSnapshotBodyV1{}, nil, nil, errors.New("snapshot_body owners must be non-empty")
	}

	adminsB64, adminPubs, err := canonicalizeEd25519PubB64Set("admin", b.Admins)
	if err != nil {
		return GovernanceSnapshotBodyV1{}, nil, nil, err
	}
	if len(adminPubs) == 0 {
		return GovernanceSnapshotBodyV1{}, nil, nil, errors.New("snapshot_body admins must be non-empty")
	}

	return GovernanceSnapshotBodyV1{
		NetID:       netID,
		PrevHashB64: prevHashB64,
		Height:      height,
		Owners:      ownersB64,
		Admins:      adminsB64,
	}, ownerPubs, adminPubs, nil
}

type GovernanceHeadSnapshotV1 struct {
	Format string `json:"format"`

	SnapshotBody GovernanceSnapshotBodyV1 `json:"snapshot_body"`
	HashB64      string                   `json:"hash_b64,omitempty"`
	Signatures   []GovernanceSignatureV1  `json:"signatures"`
}

func governanceHeadSnapshotPath(stateDir string) string {
	return filepath.Join(stateDir, "governance", "head_snapshot.json")
}

func LoadGovernanceHeadSnapshot(stateDir string) (GovernanceHeadSnapshotV1, error) {
	if strings.TrimSpace(stateDir) == "" {
		return GovernanceHeadSnapshotV1{}, errors.New("empty state_dir")
	}
	data, err := os.ReadFile(governanceHeadSnapshotPath(stateDir))
	if err != nil {
		return GovernanceHeadSnapshotV1{}, err
	}

	var h GovernanceHeadSnapshotV1
	if err := json.Unmarshal(data, &h); err != nil {
		return GovernanceHeadSnapshotV1{}, fmt.Errorf("unmarshal head_snapshot: %w", err)
	}
	if strings.TrimSpace(h.Format) == "" {
		h.Format = governanceHeadSnapshotFormatV1
	}
	if h.Format != governanceHeadSnapshotFormatV1 {
		return GovernanceHeadSnapshotV1{}, fmt.Errorf("unsupported head_snapshot format: %q", h.Format)
	}

	if err := h.ValidateSelfContained(); err != nil {
		return GovernanceHeadSnapshotV1{}, err
	}
	return h, nil
}

func SaveGovernanceHeadSnapshot(stateDir string, h GovernanceHeadSnapshotV1) error {
	if strings.TrimSpace(stateDir) == "" {
		return errors.New("empty state_dir")
	}

	h.Format = governanceHeadSnapshotFormatV1
	if err := h.ValidateSelfContained(); err != nil {
		return err
	}

	data, err := json.MarshalIndent(h, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal head_snapshot: %w", err)
	}
	data = append(data, '\n')

	path := governanceHeadSnapshotPath(stateDir)
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return fmt.Errorf("mkdir governance dir: %w", err)
	}
	if err := writeFileAtomic(path, data, 0o600); err != nil {
		return fmt.Errorf("write head_snapshot: %w", err)
	}
	return nil
}

// EnsureGovernanceHeadSnapshot ensures genesis head snapshot exists on disk.
//
// POC-06.5: only genesis creation + validation semantics.
func EnsureGovernanceHeadSnapshot(stateDir string, netID string, self Identity) (GovernanceHeadSnapshotV1, error) {
	h, err := LoadGovernanceHeadSnapshot(stateDir)
	if err == nil {
		return h, nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return GovernanceHeadSnapshotV1{}, err
	}

	netID = strings.TrimSpace(netID)
	if netID == "" {
		return GovernanceHeadSnapshotV1{}, errors.New("empty net_id")
	}

	body := GovernanceSnapshotBodyV1{
		NetID:       netID,
		PrevHashB64: "",
		Height:      0,
		Owners:      []string{self.Ed25519PubB64()},
		Admins:      []string{self.Ed25519PubB64()},
	}

	canonicalBody, hashB64, sum, ownerPubs, err := snapshotBodyHashV1(body)
	if err != nil {
		return GovernanceHeadSnapshotV1{}, err
	}
	if len(ownerPubs) != 1 {
		return GovernanceHeadSnapshotV1{}, errors.New("unexpected owner pub count")
	}

	sig := ed25519.Sign(self.Ed25519Priv, sum[:])
	h = GovernanceHeadSnapshotV1{
		Format:       governanceHeadSnapshotFormatV1,
		SnapshotBody: canonicalBody,
		HashB64:      hashB64,
		Signatures: []GovernanceSignatureV1{
			{
				KeyID:  keyIDFromEd25519Pub(ownerPubs[0]),
				SigB64: base64.RawURLEncoding.EncodeToString(sig),
			},
		},
	}

	data, err := json.MarshalIndent(h, "", "  ")
	if err != nil {
		return GovernanceHeadSnapshotV1{}, fmt.Errorf("marshal head_snapshot: %w", err)
	}
	data = append(data, '\n')

	path := governanceHeadSnapshotPath(stateDir)
	if err := writeFileExclusive(path, data, 0o600); err != nil {
		if errors.Is(err, os.ErrExist) {
			return LoadGovernanceHeadSnapshot(stateDir)
		}
		return GovernanceHeadSnapshotV1{}, fmt.Errorf("write head_snapshot: %w", err)
	}

	return h, nil
}

func (h *GovernanceHeadSnapshotV1) ValidateSelfContained() error {
	if h == nil {
		return errors.New("nil head_snapshot")
	}

	h.Format = governanceHeadSnapshotFormatV1

	canonicalBody, hashB64, sum, ownerPubs, err := snapshotBodyHashV1(h.SnapshotBody)
	if err != nil {
		return err
	}
	h.SnapshotBody = canonicalBody
	if strings.TrimSpace(h.HashB64) != "" && strings.TrimSpace(h.HashB64) != hashB64 {
		return fmt.Errorf("head_snapshot hash mismatch: have=%q want=%q", h.HashB64, hashB64)
	}
	h.HashB64 = hashB64

	sigs, err := canonicalizeSignaturesV1(h.Signatures)
	if err != nil {
		return err
	}
	h.Signatures = sigs

	ok, err := verifyOwnerThresholdV1(ownerPubs, sum, sigs, 1)
	if err != nil {
		return err
	}
	if !ok {
		return errors.New("invalid owner signatures")
	}
	return nil
}

func (h GovernanceHeadSnapshotV1) IsOwner(peerID string) bool {
	canonical, err := controlplane.CanonicalizePeerID(peerID)
	if err != nil {
		return false
	}
	pub, ok, err := h.findPeerEd25519Pub(h.SnapshotBody.Owners, canonical)
	return ok && err == nil && pub != nil
}

func (h GovernanceHeadSnapshotV1) IsAdmin(peerID string) bool {
	canonical, err := controlplane.CanonicalizePeerID(peerID)
	if err != nil {
		return false
	}
	pub, ok, err := h.findPeerEd25519Pub(h.SnapshotBody.Admins, canonical)
	return ok && err == nil && pub != nil
}

func (h GovernanceHeadSnapshotV1) AdminEd25519Pub(peerID string) (ed25519.PublicKey, bool, error) {
	canonical, err := controlplane.CanonicalizePeerID(peerID)
	if err != nil {
		return nil, false, err
	}
	return h.findPeerEd25519Pub(h.SnapshotBody.Admins, canonical)
}

func (h GovernanceHeadSnapshotV1) findPeerEd25519Pub(pubsB64 []string, peerID string) (ed25519.PublicKey, bool, error) {
	peerID = strings.TrimSpace(peerID)
	if peerID == "" {
		return nil, false, errors.New("empty peer_id")
	}

	for _, pubB64 := range pubsB64 {
		pubBytes, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(pubB64))
		if err != nil {
			return nil, false, errors.New("invalid ed25519_pub_b64")
		}
		if len(pubBytes) != ed25519.PublicKeySize {
			return nil, false, fmt.Errorf("invalid ed25519 public key length: %d", len(pubBytes))
		}
		pub := ed25519.PublicKey(pubBytes)
		derivedPeerID, err := controlplane.PeerIDFromEd25519Pub(pub)
		if err != nil {
			return nil, false, fmt.Errorf("derive peer_id from ed25519 pub: %w", err)
		}
		if derivedPeerID == peerID {
			return pub, true, nil
		}
	}
	return nil, false, nil
}

func snapshotBodyHashV1(body GovernanceSnapshotBodyV1) (GovernanceSnapshotBodyV1, string, [32]byte, []ed25519.PublicKey, error) {
	canonicalBody, ownerPubs, _, err := body.canonicalize()
	if err != nil {
		return GovernanceSnapshotBodyV1{}, "", [32]byte{}, nil, err
	}

	// canonical_json(x) = json.Marshal(x) (struct field order, no maps).
	data, err := json.Marshal(canonicalBody)
	if err != nil {
		return GovernanceSnapshotBodyV1{}, "", [32]byte{}, nil, fmt.Errorf("marshal snapshot_body: %w", err)
	}
	sum := sha256.Sum256(data)
	return canonicalBody, base64.RawURLEncoding.EncodeToString(sum[:]), sum, ownerPubs, nil
}

func canonicalizeEd25519PubB64Set(kind string, pubsB64 []string) ([]string, []ed25519.PublicKey, error) {
	type entry struct {
		b64 string
		pub ed25519.PublicKey
	}

	entries := make([]entry, 0, len(pubsB64))
	seen := make(map[string]struct{}, len(pubsB64))

	for _, raw := range pubsB64 {
		s := strings.TrimSpace(raw)
		if s == "" {
			return nil, nil, fmt.Errorf("empty %s ed25519_pub_b64", kind)
		}
		pubBytes, err := base64.RawURLEncoding.DecodeString(s)
		if err != nil {
			return nil, nil, fmt.Errorf("decode %s ed25519_pub_b64: %w", kind, err)
		}
		if len(pubBytes) != ed25519.PublicKeySize {
			return nil, nil, fmt.Errorf("invalid %s ed25519 public key length: %d", kind, len(pubBytes))
		}
		pub := ed25519.PublicKey(pubBytes)
		keyID := keyIDFromEd25519Pub(pub)
		if _, ok := seen[keyID]; ok {
			return nil, nil, fmt.Errorf("duplicate %s key_id: %s", kind, keyID)
		}
		seen[keyID] = struct{}{}

		entries = append(entries, entry{
			b64: base64.RawURLEncoding.EncodeToString(pubBytes),
			pub: pub,
		})
	}

	sort.Slice(entries, func(i, j int) bool { return entries[i].b64 < entries[j].b64 })

	outB64 := make([]string, 0, len(entries))
	outPubs := make([]ed25519.PublicKey, 0, len(entries))
	for _, e := range entries {
		outB64 = append(outB64, e.b64)
		outPubs = append(outPubs, e.pub)
	}
	return outB64, outPubs, nil
}

func canonicalizeSignaturesV1(sigs []GovernanceSignatureV1) ([]GovernanceSignatureV1, error) {
	out := make([]GovernanceSignatureV1, 0, len(sigs))
	seen := make(map[string]struct{}, len(sigs))

	for _, s := range sigs {
		keyID := strings.ToLower(strings.TrimSpace(s.KeyID))
		if keyID == "" {
			return nil, errors.New("empty signature key_id")
		}
		keyIDBytes, err := hex.DecodeString(keyID)
		if err != nil || len(keyIDBytes) != sha256.Size {
			return nil, fmt.Errorf("invalid signature key_id: %q", s.KeyID)
		}

		sigB64 := strings.TrimSpace(s.SigB64)
		if sigB64 == "" {
			return nil, errors.New("empty signature sig_b64")
		}
		if _, err := base64.RawURLEncoding.DecodeString(sigB64); err != nil {
			return nil, fmt.Errorf("invalid signature sig_b64: %w", err)
		}

		if _, ok := seen[keyID]; ok {
			return nil, fmt.Errorf("duplicate signature key_id: %s", keyID)
		}
		seen[keyID] = struct{}{}

		out = append(out, GovernanceSignatureV1{
			KeyID:  keyID,
			SigB64: sigB64,
		})
	}

	sort.Slice(out, func(i, j int) bool { return out[i].KeyID < out[j].KeyID })
	return out, nil
}

func verifyOwnerThresholdV1(owners []ed25519.PublicKey, bodySum [32]byte, sigs []GovernanceSignatureV1, threshold int) (bool, error) {
	if threshold <= 0 {
		return true, nil
	}

	ownersByKeyID := make(map[string]ed25519.PublicKey, len(owners))
	for _, pub := range owners {
		keyID := keyIDFromEd25519Pub(pub)
		if _, ok := ownersByKeyID[keyID]; ok {
			return false, fmt.Errorf("duplicate owner key_id: %s", keyID)
		}
		ownersByKeyID[keyID] = pub
	}

	valid := 0
	validSeen := make(map[string]struct{}, len(sigs))

	for _, s := range sigs {
		pub, ok := ownersByKeyID[strings.ToLower(strings.TrimSpace(s.KeyID))]
		if !ok {
			continue
		}

		sigBytes, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(s.SigB64))
		if err != nil {
			return false, fmt.Errorf("decode signature sig_b64: %w", err)
		}
		if len(sigBytes) != ed25519.SignatureSize {
			return false, fmt.Errorf("invalid signature length: %d", len(sigBytes))
		}
		if !ed25519.Verify(pub, bodySum[:], sigBytes) {
			continue
		}
		if _, ok := validSeen[s.KeyID]; ok {
			continue
		}
		validSeen[s.KeyID] = struct{}{}
		valid++
		if valid >= threshold {
			return true, nil
		}
	}
	return false, nil
}

func keyIDFromEd25519Pub(pub ed25519.PublicKey) string {
	sum := sha256.Sum256(pub)
	return hex.EncodeToString(sum[:])
}

func validateNetID(netID string) error {
	if len(netID) != 26 {
		return fmt.Errorf("invalid net_id length: %d", len(netID))
	}
	for i := 0; i < len(netID); i++ {
		c := netID[i]
		if c >= 'A' && c <= 'Z' {
			continue
		}
		if c >= '2' && c <= '7' {
			continue
		}
		return fmt.Errorf("invalid net_id character: %q", c)
	}
	return nil
}

func ValidateGovernanceHeadSnapshotBootstrap(localNetID string, head GovernanceHeadSnapshotV1) error {
	localNetID = strings.TrimSpace(localNetID)
	if localNetID == "" {
		return errors.New("empty local net_id")
	}
	if err := head.ValidateSelfContained(); err != nil {
		return err
	}
	if head.SnapshotBody.NetID != localNetID {
		return fmt.Errorf("snapshot_body net_id mismatch: have=%q want=%q", head.SnapshotBody.NetID, localNetID)
	}
	return nil
}

func ValidateGovernanceHeadSnapshotUpdate(localNetID string, localHead GovernanceHeadSnapshotV1, candidate GovernanceHeadSnapshotV1) error {
	localNetID = strings.TrimSpace(localNetID)
	if localNetID == "" {
		return errors.New("empty local net_id")
	}

	if err := localHead.ValidateSelfContained(); err != nil {
		return fmt.Errorf("invalid local head snapshot: %w", err)
	}
	if err := candidate.ValidateSelfContained(); err != nil {
		return fmt.Errorf("invalid candidate head snapshot: %w", err)
	}

	localHash := strings.TrimSpace(localHead.HashB64)
	candidateHash := strings.TrimSpace(candidate.HashB64)
	if localHash != "" && candidateHash == localHash {
		return nil
	}

	if localHead.SnapshotBody.NetID != localNetID {
		return fmt.Errorf("local head net_id mismatch: have=%q want=%q", localHead.SnapshotBody.NetID, localNetID)
	}
	if candidate.SnapshotBody.NetID != localNetID {
		return fmt.Errorf("candidate net_id mismatch: have=%q want=%q", candidate.SnapshotBody.NetID, localNetID)
	}

	if strings.TrimSpace(candidate.SnapshotBody.PrevHashB64) != localHash {
		return errors.New("candidate prev_hash_b64 does not match local head hash")
	}
	if candidate.SnapshotBody.Height != localHead.SnapshotBody.Height+1 {
		return fmt.Errorf("candidate height mismatch: have=%d want=%d", candidate.SnapshotBody.Height, localHead.SnapshotBody.Height+1)
	}

	_, _, candidateSum, _, err := snapshotBodyHashV1(candidate.SnapshotBody)
	if err != nil {
		return err
	}
	_, localOwnerPubs, err := canonicalizeEd25519PubB64Set("owner", localHead.SnapshotBody.Owners)
	if err != nil {
		return err
	}

	oldOK, err := verifyOwnerThresholdV1(localOwnerPubs, candidateSum, candidate.Signatures, 1)
	if err != nil {
		return err
	}
	if !oldOK {
		return errors.New("candidate does not satisfy old-threshold")
	}

	return nil
}
