package task

import (
	"crypto/rand"
	"encoding/base32"
	"encoding/base64"
	"fmt"
	"strings"
)

var base32RawNoPad = base32.StdEncoding.WithPadding(base32.NoPadding)

func newRandomTopic() (string, error) {
	// 16B -> base32(raw,no-pad) -> 26 chars.
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return "", fmt.Errorf("read random: %w", err)
	}
	return strings.ToLower(base32RawNoPad.EncodeToString(b)), nil
}

func newSecretKeyB64URLNoPad() (string, []byte, error) {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return "", nil, fmt.Errorf("read random: %w", err)
	}
	return base64.RawURLEncoding.EncodeToString(b), b, nil
}
