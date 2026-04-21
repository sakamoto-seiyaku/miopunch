package controlplane

import (
	"crypto/rand"
	"encoding/base32"
	"errors"
	"fmt"
	"strings"
	"unicode"
)

const (
	msgIDRawLen = 16 // 16 bytes -> base32(raw,no-pad) -> 26 chars
	msgIDLen    = 26
)

func NewMsgID() (string, error) {
	b := make([]byte, msgIDRawLen)
	if _, err := rand.Read(b); err != nil {
		return "", fmt.Errorf("new msg_id: %w", err)
	}
	return base32RawNoPad.EncodeToString(b), nil
}

// CanonicalizeMsgID normalizes a message ID into its canonical wire form:
// uppercase base32(raw,no-pad), fixed length, no separators.
func CanonicalizeMsgID(value string) (string, error) {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" {
		return "", errors.New("empty msg_id")
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
	if len(canonical) != msgIDLen {
		return "", fmt.Errorf("invalid msg_id length: %d", len(canonical))
	}

	for _, r := range canonical {
		if r >= 'A' && r <= 'Z' {
			continue
		}
		if r >= '2' && r <= '7' {
			continue
		}
		return "", fmt.Errorf("invalid msg_id character: %q", r)
	}

	decoded, err := base32.StdEncoding.WithPadding(base32.NoPadding).DecodeString(canonical)
	if err != nil {
		return "", fmt.Errorf("invalid msg_id base32: %w", err)
	}
	if len(decoded) != msgIDRawLen {
		return "", fmt.Errorf("invalid msg_id decoded length: %d", len(decoded))
	}

	return canonical, nil
}
