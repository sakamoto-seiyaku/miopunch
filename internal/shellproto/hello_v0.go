package shellproto

import (
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

const (
	ReasonHelloRequired       = "HELLO_REQUIRED"
	ReasonHelloInvalid        = "HELLO_INVALID"
	ReasonHelloNotApproved    = "HELLO_NOT_APPROVED"
	ReasonHelloRevoked        = "HELLO_REVOKED"
	ReasonHelloIssuerNotAdmin = "HELLO_ISSUER_NOT_ADMIN"
	ReasonHelloDeclInvalid    = "HELLO_DECL_INVALID"
	ReasonHelloSigInvalid     = "HELLO_SIG_INVALID"
	ReasonHelloInternal       = "HELLO_INTERNAL"
)

var (
	ErrHelloInvalidSignature = errors.New("invalid hello signature")
)

type helloTranscriptV0 struct {
	PeerID       string `json:"peer_id"`
	ApproveMsgID string `json:"approve_msg_id,omitempty"`
}

func helloTranscriptSumV0(peerID string, approveMsgID string) ([32]byte, error) {
	peerID = strings.TrimSpace(peerID)
	if peerID == "" {
		return [32]byte{}, errors.New("empty peer_id")
	}
	approveMsgID = strings.TrimSpace(approveMsgID)

	t := helloTranscriptV0{PeerID: peerID}
	if approveMsgID != "" {
		t.ApproveMsgID = approveMsgID
	}

	data, err := json.Marshal(t)
	if err != nil {
		return [32]byte{}, fmt.Errorf("marshal hello transcript: %w", err)
	}
	return sha256.Sum256(data), nil
}

// SignHelloV0 signs the POC v0 hello handshake transcript.
func SignHelloV0(priv ed25519.PrivateKey, peerID string, approveMsgID string) (string, error) {
	if len(priv) != ed25519.PrivateKeySize {
		return "", fmt.Errorf("invalid ed25519 priv length: %d", len(priv))
	}
	sum, err := helloTranscriptSumV0(peerID, approveMsgID)
	if err != nil {
		return "", err
	}
	sig := ed25519.Sign(priv, sum[:])
	return base64.RawURLEncoding.EncodeToString(sig), nil
}

// VerifyHelloV0 verifies the POC v0 hello handshake transcript signature.
func VerifyHelloV0(pub ed25519.PublicKey, peerID string, approveMsgID string, sigB64 string) error {
	if len(pub) != ed25519.PublicKeySize {
		return fmt.Errorf("invalid ed25519 pub length: %d", len(pub))
	}
	sig, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(sigB64))
	if err != nil {
		return fmt.Errorf("decode sig_b64: %w", err)
	}
	if len(sig) != ed25519.SignatureSize {
		return fmt.Errorf("invalid signature length: %d", len(sig))
	}

	sum, err := helloTranscriptSumV0(peerID, approveMsgID)
	if err != nil {
		return err
	}
	if !ed25519.Verify(pub, sum[:], sig) {
		return ErrHelloInvalidSignature
	}
	return nil
}
