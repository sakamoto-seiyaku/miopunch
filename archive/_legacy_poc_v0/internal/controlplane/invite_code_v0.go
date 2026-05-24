package controlplane

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"strings"
	"time"

	"github.com/btcsuite/btcd/btcutil/bech32"
)

const (
	inviteCodeHRPV0 = "miopunch"

	inviteCodeTypeJoin  = "join"
	inviteCodeVersionV0 = 0

	// POC v0 hard limit (including separators).
	inviteCodeMaxLen = 1024

	inviteCodeURLScheme = "miopunch"
	inviteCodeURLHost   = "join"
)

type InviteMode string

const (
	InviteModeApprove InviteMode = "approve"
	InviteModeAuto    InviteMode = "auto"
)

// InviteCodeV0 is the decoded join invitation payload (before bech32m encoding).
type InviteCodeV0 struct {
	CodeType string `json:"code_type"`
	Version  int    `json:"version"`

	IssuerPeerID        string `json:"issuer_peer_id"`
	IssuerEd25519PubB64 string `json:"issuer_ed25519_pub_b64"`
	IssuerX25519PubB64  string `json:"issuer_x25519_pub_b64"`

	InviteBrokers   []string   `json:"invite_brokers"`
	InviteTopic     string     `json:"invite_topic"`
	InviteSecretB64 string     `json:"invite_secret_b64"`
	Mode            InviteMode `json:"mode"`
	MaxUses         int        `json:"max_uses"`
	ExpiresAtUnixMs int64      `json:"expires_at_unix_ms"`
}

func (c InviteCodeV0) Validate() error {
	if strings.TrimSpace(c.CodeType) == "" {
		c.CodeType = inviteCodeTypeJoin
	}
	if c.CodeType != inviteCodeTypeJoin {
		return fmt.Errorf("invalid code_type: %q", c.CodeType)
	}
	if c.Version == 0 {
		c.Version = inviteCodeVersionV0
	}
	if c.Version != inviteCodeVersionV0 {
		return fmt.Errorf("invalid code version: %d", c.Version)
	}

	if _, err := CanonicalizePeerID(c.IssuerPeerID); err != nil {
		return fmt.Errorf("invalid issuer_peer_id: %w", err)
	}

	if _, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(c.IssuerEd25519PubB64)); err != nil {
		return fmt.Errorf("invalid issuer_ed25519_pub_b64: %w", err)
	}
	issuerXPub, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(c.IssuerX25519PubB64))
	if err != nil {
		return fmt.Errorf("invalid issuer_x25519_pub_b64: %w", err)
	}
	if len(issuerXPub) != 32 {
		return fmt.Errorf("invalid issuer x25519 pub length: %d", len(issuerXPub))
	}

	if err := validateInviteBrokers(c.InviteBrokers); err != nil {
		return err
	}

	if strings.TrimSpace(c.InviteTopic) == "" {
		return errors.New("invite_topic is required")
	}

	secret, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(c.InviteSecretB64))
	if err != nil {
		return fmt.Errorf("invalid invite_secret_b64: %w", err)
	}
	if len(secret) != 32 {
		return fmt.Errorf("invalid invite_secret length: %d", len(secret))
	}

	switch strings.TrimSpace(string(c.Mode)) {
	case string(InviteModeApprove), string(InviteModeAuto):
	default:
		return fmt.Errorf("invalid mode: %q", c.Mode)
	}

	if c.MaxUses <= 0 {
		return errors.New("max_uses must be > 0")
	}
	if c.ExpiresAtUnixMs <= 0 {
		return errors.New("expires_at_unix_ms must be > 0")
	}
	if time.Now().UTC().UnixMilli() > c.ExpiresAtUnixMs {
		return errors.New("invite already expired")
	}

	return nil
}

func EncodeInviteCodeV0(c InviteCodeV0) (string, error) {
	c.CodeType = strings.TrimSpace(c.CodeType)
	if c.CodeType == "" {
		c.CodeType = inviteCodeTypeJoin
	}
	if c.Version == 0 {
		c.Version = inviteCodeVersionV0
	}

	if err := c.Validate(); err != nil {
		return "", err
	}

	data, err := json.Marshal(c)
	if err != nil {
		return "", fmt.Errorf("marshal invite code: %w", err)
	}

	data5, err := bech32.ConvertBits(data, 8, 5, true)
	if err != nil {
		return "", fmt.Errorf("bech32 convert bits: %w", err)
	}

	code, err := bech32.EncodeM(inviteCodeHRPV0, data5)
	if err != nil {
		return "", fmt.Errorf("bech32 encode: %w", err)
	}
	return strings.ToLower(code), nil
}

func DecodeInviteCodeV0(codeOrURL string) (InviteCodeV0, error) {
	raw := strings.TrimSpace(codeOrURL)
	if raw == "" {
		return InviteCodeV0{}, errors.New("empty invite code")
	}

	normalized, err := normalizeInviteCodeInput(raw)
	if err != nil {
		return InviteCodeV0{}, err
	}

	hrp, data5, err := bech32.DecodeNoLimit(normalized)
	if err != nil {
		return InviteCodeV0{}, fmt.Errorf("bech32 decode: %w", err)
	}
	if strings.ToLower(hrp) != inviteCodeHRPV0 {
		return InviteCodeV0{}, fmt.Errorf("invalid invite code hrp: %q", hrp)
	}

	data, err := bech32.ConvertBits(data5, 5, 8, false)
	if err != nil {
		return InviteCodeV0{}, fmt.Errorf("bech32 convert bits: %w", err)
	}

	var c InviteCodeV0
	if err := json.Unmarshal(data, &c); err != nil {
		return InviteCodeV0{}, fmt.Errorf("unmarshal invite code: %w", err)
	}
	if strings.TrimSpace(c.CodeType) == "" {
		c.CodeType = inviteCodeTypeJoin
	}
	if c.Version == 0 {
		c.Version = inviteCodeVersionV0
	}

	if err := c.Validate(); err != nil {
		return InviteCodeV0{}, err
	}
	return c, nil
}

func normalizeInviteCodeInput(raw string) (string, error) {
	trimmed := strings.TrimSpace(raw)
	if trimmed == "" {
		return "", errors.New("empty invite code")
	}

	// Minimal URL form: miopunch://join/<code>
	if strings.HasPrefix(trimmed, inviteCodeURLScheme+"://") {
		u, err := url.Parse(trimmed)
		if err != nil {
			return "", fmt.Errorf("invalid invite url: %w", err)
		}
		if u.Scheme != inviteCodeURLScheme {
			return "", errors.New("invalid invite url scheme")
		}
		if u.Host != inviteCodeURLHost {
			return "", errors.New("invalid invite url host")
		}
		payload := strings.Trim(u.Path, "/")
		if payload == "" {
			payload = u.Query().Get("code")
		}
		if payload == "" {
			return "", errors.New("missing invite url payload")
		}
		trimmed = payload
	}

	// Remove separators (spaces, hyphens).
	var b strings.Builder
	b.Grow(len(trimmed))
	for _, r := range trimmed {
		switch r {
		case '-', ' ', '\t', '\n', '\r':
			continue
		default:
			b.WriteRune(r)
		}
	}
	code := strings.ToLower(b.String())
	if len(code) > inviteCodeMaxLen {
		return "", fmt.Errorf("invite code too long: %d", len(code))
	}
	return code, nil
}
