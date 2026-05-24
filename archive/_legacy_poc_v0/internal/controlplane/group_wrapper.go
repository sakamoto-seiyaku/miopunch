package controlplane

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/hkdf"
	"crypto/rand"
	"crypto/sha256"
	"errors"
	"fmt"
)

const (
	groupWrapperInfoV0 = "miopunch/v0/aead.ctrl.group"

	groupWrapperVersionV0 = 0
	groupWrapperNonceLen  = 12
)

var (
	ErrInvalidCiphertextFrame       = errors.New("invalid ciphertext frame")
	ErrUnsupportedCiphertextVersion = errors.New("unsupported ciphertext version")
)

func deriveGroupWrapperKeyV0(netSecret []byte) ([]byte, error) {
	if len(netSecret) == 0 {
		return nil, errors.New("net_secret is required")
	}
	netIDRaw := sha256.Sum256(netSecret)
	key, err := hkdf.Key(sha256.New, netSecret, netIDRaw[:16], groupWrapperInfoV0, 32)
	if err != nil {
		return nil, fmt.Errorf("derive group wrapper key: %w", err)
	}
	if len(key) != 32 {
		return nil, fmt.Errorf("unexpected group wrapper key length: %d", len(key))
	}
	return key, nil
}

func SealGroupV0(netSecret []byte, plaintext []byte) ([]byte, error) {
	key, err := deriveGroupWrapperKeyV0(netSecret)
	if err != nil {
		return nil, err
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, fmt.Errorf("new cipher: %w", err)
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, fmt.Errorf("new gcm: %w", err)
	}
	if gcm.NonceSize() != groupWrapperNonceLen {
		return nil, fmt.Errorf("unexpected gcm nonce size: %d", gcm.NonceSize())
	}

	nonce := make([]byte, groupWrapperNonceLen)
	if _, err := rand.Read(nonce); err != nil {
		return nil, fmt.Errorf("read nonce: %w", err)
	}
	ct := gcm.Seal(nil, nonce, plaintext, nil)

	frame := make([]byte, 1+groupWrapperNonceLen+len(ct))
	frame[0] = groupWrapperVersionV0
	copy(frame[1:], nonce)
	copy(frame[1+groupWrapperNonceLen:], ct)
	return frame, nil
}

func OpenGroupV0(netSecret []byte, frame []byte) ([]byte, error) {
	if len(frame) < 1+groupWrapperNonceLen+16 {
		return nil, ErrInvalidCiphertextFrame
	}

	if frame[0] != groupWrapperVersionV0 {
		return nil, ErrUnsupportedCiphertextVersion
	}
	nonce := frame[1 : 1+groupWrapperNonceLen]
	ct := frame[1+groupWrapperNonceLen:]

	key, err := deriveGroupWrapperKeyV0(netSecret)
	if err != nil {
		return nil, err
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, fmt.Errorf("new cipher: %w", err)
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, fmt.Errorf("new gcm: %w", err)
	}
	if gcm.NonceSize() != groupWrapperNonceLen {
		return nil, fmt.Errorf("unexpected gcm nonce size: %d", gcm.NonceSize())
	}

	pt, err := gcm.Open(nil, nonce, ct, nil)
	if err != nil {
		return nil, fmt.Errorf("open gcm: %w", err)
	}
	return pt, nil
}
