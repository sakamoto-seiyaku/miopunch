package pocstate

import (
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base32"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"golang.org/x/crypto/curve25519"
)

const (
	identityFormatV0 = "miopunch.identity.poc.v0"
)

var (
	base32RawNoPadIdentity = base32.StdEncoding.WithPadding(base32.NoPadding)
)

type identityRecordV0 struct {
	Format          string `json:"format"`
	CreatedAtUnixMs int64  `json:"created_at_unix_ms"`

	PeerID string `json:"peer_id"`

	Ed25519SeedB64 string `json:"ed25519_seed_b64"` // base64url(no-pad), 32B
	X25519PrivB64  string `json:"x25519_priv_b64"`  // base64url(no-pad), 32B
}

// Identity is the loaded long-term node identity (POC v0).
//
// The identity is persisted under state_dir/identity/identity.json.
type Identity struct {
	PeerID string

	Ed25519Priv ed25519.PrivateKey
	Ed25519Pub  ed25519.PublicKey

	X25519Priv []byte // 32B
	X25519Pub  []byte // 32B
}

func (i Identity) Ed25519PubB64() string {
	return base64.RawURLEncoding.EncodeToString(i.Ed25519Pub)
}

func (i Identity) X25519PubB64() string {
	return base64.RawURLEncoding.EncodeToString(i.X25519Pub)
}

func identityPath(stateDir string) string {
	return filepath.Join(stateDir, "identity", "identity.json")
}

func EnsureIdentity(stateDir string) (Identity, error) {
	if strings.TrimSpace(stateDir) == "" {
		return Identity{}, errors.New("empty state_dir")
	}

	path := identityPath(stateDir)
	rec, err := loadIdentityRecord(path)
	if err == nil {
		return identityFromRecord(rec)
	}
	if !errors.Is(err, os.ErrNotExist) {
		return Identity{}, err
	}

	rec, id, err := newIdentityRecord()
	if err != nil {
		return Identity{}, err
	}

	data, err := json.MarshalIndent(rec, "", "  ")
	if err != nil {
		return Identity{}, fmt.Errorf("marshal identity: %w", err)
	}
	data = append(data, '\n')

	if err := writeFileExclusive(path, data, 0o600); err != nil {
		if errors.Is(err, os.ErrExist) {
			// Lost the race to create the file; load it.
			rec, err := loadIdentityRecord(path)
			if err != nil {
				return Identity{}, err
			}
			return identityFromRecord(rec)
		}
		return Identity{}, err
	}

	return id, nil
}

func loadIdentityRecord(path string) (identityRecordV0, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return identityRecordV0{}, err
	}
	var rec identityRecordV0
	if err := json.Unmarshal(data, &rec); err != nil {
		return identityRecordV0{}, fmt.Errorf("unmarshal identity: %w", err)
	}
	if strings.TrimSpace(rec.Format) == "" {
		rec.Format = identityFormatV0
	}
	if rec.Format != identityFormatV0 {
		return identityRecordV0{}, fmt.Errorf("unsupported identity format: %q", rec.Format)
	}
	return rec, nil
}

func newIdentityRecord() (identityRecordV0, Identity, error) {
	seed := make([]byte, 32)
	if _, err := rand.Read(seed); err != nil {
		return identityRecordV0{}, Identity{}, fmt.Errorf("read ed25519 seed: %w", err)
	}
	edPriv := ed25519.NewKeyFromSeed(seed)
	edPub := edPriv.Public().(ed25519.PublicKey)

	xpriv := make([]byte, 32)
	if _, err := rand.Read(xpriv); err != nil {
		return identityRecordV0{}, Identity{}, fmt.Errorf("read x25519 priv: %w", err)
	}
	xpub, err := curve25519.X25519(xpriv, curve25519.Basepoint)
	if err != nil {
		return identityRecordV0{}, Identity{}, fmt.Errorf("x25519 pub: %w", err)
	}

	peerID, err := peerIDFromEd25519Pub(edPub)
	if err != nil {
		return identityRecordV0{}, Identity{}, err
	}

	id := Identity{
		PeerID:      peerID,
		Ed25519Priv: edPriv,
		Ed25519Pub:  edPub,
		X25519Priv:  xpriv,
		X25519Pub:   xpub,
	}

	rec := identityRecordV0{
		Format:          identityFormatV0,
		CreatedAtUnixMs: time.Now().UTC().UnixMilli(),
		PeerID:          peerID,
		Ed25519SeedB64:  base64.RawURLEncoding.EncodeToString(seed),
		X25519PrivB64:   base64.RawURLEncoding.EncodeToString(xpriv),
	}
	return rec, id, nil
}

func identityFromRecord(rec identityRecordV0) (Identity, error) {
	seed, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(rec.Ed25519SeedB64))
	if err != nil {
		return Identity{}, fmt.Errorf("decode ed25519_seed_b64: %w", err)
	}
	if len(seed) != 32 {
		return Identity{}, fmt.Errorf("invalid ed25519 seed length: %d", len(seed))
	}
	edPriv := ed25519.NewKeyFromSeed(seed)
	edPub := edPriv.Public().(ed25519.PublicKey)

	xpriv, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(rec.X25519PrivB64))
	if err != nil {
		return Identity{}, fmt.Errorf("decode x25519_priv_b64: %w", err)
	}
	if len(xpriv) != 32 {
		return Identity{}, fmt.Errorf("invalid x25519 priv length: %d", len(xpriv))
	}
	xpub, err := curve25519.X25519(xpriv, curve25519.Basepoint)
	if err != nil {
		return Identity{}, fmt.Errorf("x25519 pub: %w", err)
	}

	peerID, err := peerIDFromEd25519Pub(edPub)
	if err != nil {
		return Identity{}, err
	}
	if strings.TrimSpace(rec.PeerID) != "" && peerID != strings.TrimSpace(rec.PeerID) {
		return Identity{}, fmt.Errorf("identity peer_id mismatch: have=%q want=%q", rec.PeerID, peerID)
	}

	return Identity{
		PeerID:      peerID,
		Ed25519Priv: edPriv,
		Ed25519Pub:  edPub,
		X25519Priv:  xpriv,
		X25519Pub:   xpub,
	}, nil
}

func peerIDFromEd25519Pub(pub ed25519.PublicKey) (string, error) {
	if len(pub) != ed25519.PublicKeySize {
		return "", fmt.Errorf("invalid ed25519 public key length: %d", len(pub))
	}
	sum := sha256.Sum256(pub)
	raw := sum[:16]
	peerID := strings.ToUpper(base32RawNoPadIdentity.EncodeToString(raw))
	if len(peerID) != 26 {
		return "", fmt.Errorf("unexpected peer_id length: %d", len(peerID))
	}
	return peerID, nil
}

func writeFileExclusive(path string, data []byte, perm os.FileMode) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("mkdir: %w", err)
	}

	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, perm)
	if err != nil {
		return err
	}
	defer func() { _ = f.Close() }()

	if _, err := f.Write(data); err != nil {
		_ = f.Close()
		_ = os.Remove(path)
		return fmt.Errorf("write: %w", err)
	}
	if err := f.Sync(); err != nil {
		_ = f.Close()
		_ = os.Remove(path)
		return fmt.Errorf("fsync: %w", err)
	}
	if err := f.Close(); err != nil {
		_ = os.Remove(path)
		return fmt.Errorf("close: %w", err)
	}

	// Best effort: fsync parent directory for crash-safety on Unix.
	if d, err := os.Open(dir); err == nil {
		_ = d.Sync()
		_ = d.Close()
	}

	return nil
}
