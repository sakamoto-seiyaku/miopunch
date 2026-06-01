package controlplane

import (
	"encoding/base32"
	"errors"
	"fmt"
	"strings"
	"unicode"
)

const (
	peerIDLen = 26 // 16 bytes -> base32(raw,no-pad) -> 26 chars
)

// CanonicalizePeerID normalizes a peer ID into its canonical wire form:
// uppercase base32(raw,no-pad), fixed length, no separators.
func CanonicalizePeerID(value string) (string, error) {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" {
		return "", errors.New("empty peer_id")
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
	if len(canonical) != peerIDLen {
		return "", fmt.Errorf("invalid peer_id length: %d", len(canonical))
	}

	for _, r := range canonical {
		if r >= 'A' && r <= 'Z' {
			continue
		}
		if r >= '2' && r <= '7' {
			continue
		}
		return "", fmt.Errorf("invalid peer_id character: %q", r)
	}

	decoded, err := base32.StdEncoding.WithPadding(base32.NoPadding).DecodeString(canonical)
	if err != nil {
		return "", fmt.Errorf("invalid peer_id base32: %w", err)
	}
	if len(decoded) != 16 {
		return "", fmt.Errorf("invalid peer_id decoded length: %d", len(decoded))
	}

	return canonical, nil
}
