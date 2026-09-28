package poc

import (
	"crypto/rand"
	"encoding/base32"
	"errors"
	"fmt"
	"strings"
	"unicode"
)

const (
	idRawLen = 16 // 16 bytes -> base32(raw,no-pad) -> 26 chars
	idLen    = 26
)

var base32RawNoPad = base32.StdEncoding.WithPadding(base32.NoPadding)

func NewTaskID() (string, error) {
	return newID("task_id")
}

func CanonicalizeTaskID(value string) (string, error) {
	return canonicalizeID(value, "task_id")
}

func NewRequestID() (string, error) {
	return newID("request_id")
}

func CanonicalizeRequestID(value string) (string, error) {
	return canonicalizeID(value, "request_id")
}

func newID(name string) (string, error) {
	b := make([]byte, idRawLen)
	if _, err := rand.Read(b); err != nil {
		return "", fmt.Errorf("new %s: %w", name, err)
	}
	return base32RawNoPad.EncodeToString(b), nil
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
	if len(canonical) != idLen {
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
	if len(decoded) != idRawLen {
		return "", errors.New("invalid decoded length")
	}
	return canonical, nil
}
