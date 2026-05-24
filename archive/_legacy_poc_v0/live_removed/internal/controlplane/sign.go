package controlplane

import (
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
)

var (
	ErrInvalidSignature = errors.New("invalid signature")
	ErrNotForSelf       = errors.New("message dst_peer_id does not match self_peer_id")
)

type transcriptV0 struct {
	DstPeerID       string `json:"dst_peer_id"`
	MsgID           string `json:"msg_id"`
	CreatedAtUnixMs int64  `json:"created_at_unix_ms"`
	ExpiresAtUnixMs int64  `json:"expires_at_unix_ms,omitempty"`

	SenderPeerID string          `json:"sender_peer_id"`
	Kind         string          `json:"kind"`
	InReplyTo    string          `json:"in_reply_to,omitempty"`
	Body         json.RawMessage `json:"body"`
}

func buildTranscriptV0(m Message) ([]byte, error) {
	t := transcriptV0{
		DstPeerID:       m.Route.DstPeerID,
		MsgID:           m.Route.MsgID,
		CreatedAtUnixMs: m.Route.CreatedAtUnixMs,
		ExpiresAtUnixMs: m.Route.ExpiresAtUnixMs,
		SenderPeerID:    m.Signed.SenderPeerID,
		Kind:            m.Signed.Kind,
		InReplyTo:       m.Signed.InReplyTo,
		Body:            m.Signed.Body,
	}
	data, err := json.Marshal(t)
	if err != nil {
		return nil, fmt.Errorf("build transcript: %w", err)
	}
	return data, nil
}

func SignV0(priv ed25519.PrivateKey, m *Message) error {
	if m == nil {
		return errors.New("nil message")
	}
	if len(priv) != ed25519.PrivateKeySize {
		return fmt.Errorf("invalid ed25519 private key length: %d", len(priv))
	}

	transcriptJSON, err := buildTranscriptV0(*m)
	if err != nil {
		return err
	}
	sum := sha256.Sum256(transcriptJSON)
	sig := ed25519.Sign(priv, sum[:])
	m.Signed.SigB64 = base64URLNoPad.EncodeToString(sig)
	return nil
}

func VerifyV0(pub ed25519.PublicKey, m Message) error {
	if len(pub) != ed25519.PublicKeySize {
		return fmt.Errorf("invalid ed25519 public key length: %d", len(pub))
	}

	sig, err := base64URLNoPad.DecodeString(m.Signed.SigB64)
	if err != nil {
		return fmt.Errorf("decode sig_b64: %w", err)
	}
	transcriptJSON, err := buildTranscriptV0(m)
	if err != nil {
		return err
	}
	sum := sha256.Sum256(transcriptJSON)
	if !ed25519.Verify(pub, sum[:], sig) {
		return ErrInvalidSignature
	}
	return nil
}

func VerifyV0ForSelf(selfPeerID string, pub ed25519.PublicKey, m Message) error {
	if err := VerifyV0(pub, m); err != nil {
		return err
	}
	if m.Route.DstPeerID != selfPeerID {
		return ErrNotForSelf
	}
	return nil
}
