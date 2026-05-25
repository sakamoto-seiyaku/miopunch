// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

package wire

import (
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base32"
	"errors"
	"fmt"
	"strings"
	"unicode"
)

const (
	// RawIDLen is the raw 16-byte length shared by canonical msg_id, peer_id,
	// and network_id values.
	RawIDLen = 16
	// CanonicalIDLen is the fixed uppercase base32 string length for raw 16-byte
	// identifiers without padding.
	CanonicalIDLen = 26
)

var base32RawNoPad = base32.StdEncoding.WithPadding(base32.NoPadding)

// NewMsgID returns a new canonical current v1 msg_id.
func NewMsgID() (string, error) {
	b := make([]byte, RawIDLen)
	if _, err := rand.Read(b); err != nil {
		return "", fmt.Errorf("new msg_id: %w", err)
	}
	return base32RawNoPad.EncodeToString(b), nil
}

// CanonicalizeMsgID normalizes a current v1 msg_id into its wire form.
func CanonicalizeMsgID(value string) (string, error) {
	return canonicalizeID(value, "msg_id")
}

// CanonicalizePeerID normalizes a current v1 peer_id into its wire form.
func CanonicalizePeerID(value string) (string, error) {
	return canonicalizeID(value, "peer_id")
}

// CanonicalizeNetworkID normalizes a current v1 network_id into its wire form.
func CanonicalizeNetworkID(value string) (string, error) {
	return canonicalizeID(value, "network_id")
}

// EncodeNetworkID converts a raw 16-byte network identifier into canonical
// uppercase base32 text.
func EncodeNetworkID(raw []byte) (string, error) {
	if len(raw) != RawIDLen {
		return "", fmt.Errorf("invalid network_id_bytes length: %d", len(raw))
	}
	return base32RawNoPad.EncodeToString(raw), nil
}

// DecodeNetworkID converts a canonical network_id string into raw 16-byte form.
func DecodeNetworkID(value string) ([]byte, error) {
	canonical, err := CanonicalizeNetworkID(value)
	if err != nil {
		return nil, err
	}

	raw, err := base32RawNoPad.DecodeString(canonical)
	if err != nil {
		return nil, fmt.Errorf("decode network_id: %w", err)
	}
	return raw, nil
}

// PeerIDFromEd25519Pub derives the canonical peer_id from an Ed25519 public
// key using the legacy v0 hash-based derivation.
func PeerIDFromEd25519Pub(pub ed25519.PublicKey) (string, error) {
	if len(pub) != ed25519.PublicKeySize {
		return "", fmt.Errorf("invalid ed25519 public key length: %d", len(pub))
	}

	sum := sha256.Sum256(pub)
	return base32RawNoPad.EncodeToString(sum[:RawIDLen]), nil
}

func canonicalizeID(value string, name string) (string, error) {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" {
		return "", fmt.Errorf("empty %s", name)
	}

	var b strings.Builder
	b.Grow(len(trimmed))
	for _, r := range trimmed {
		if r == '-' || unicode.IsSpace(r) {
			continue
		}
		b.WriteRune(r)
	}

	canonical := strings.ToUpper(b.String())
	if len(canonical) != CanonicalIDLen {
		return "", fmt.Errorf("invalid %s length: %d", name, len(canonical))
	}

	for _, r := range canonical {
		if r >= 'A' && r <= 'Z' {
			continue
		}
		if r >= '2' && r <= '7' {
			continue
		}
		return "", fmt.Errorf("invalid %s character: %q", name, r)
	}

	decoded, err := base32RawNoPad.DecodeString(canonical)
	if err != nil {
		return "", fmt.Errorf("invalid %s base32: %w", name, err)
	}
	if len(decoded) != RawIDLen {
		return "", errors.New("invalid decoded length")
	}

	return canonical, nil
}
