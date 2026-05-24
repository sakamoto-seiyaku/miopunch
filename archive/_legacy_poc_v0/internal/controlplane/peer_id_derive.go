package controlplane

import (
	"crypto/ed25519"
	"crypto/sha256"
	"fmt"
)

// PeerIDFromEd25519Pub derives the canonical peer_id from an Ed25519 signing
// public key:
//
// peer_id = base32(raw,no-pad, sha256(ed25519_pub)[:16]) (26 chars, upper-case).
func PeerIDFromEd25519Pub(pub ed25519.PublicKey) (string, error) {
	if len(pub) != ed25519.PublicKeySize {
		return "", fmt.Errorf("invalid ed25519 public key length: %d", len(pub))
	}
	sum := sha256.Sum256(pub)
	raw := sum[:16]
	id := base32RawNoPad.EncodeToString(raw)
	return CanonicalizePeerID(id)
}
