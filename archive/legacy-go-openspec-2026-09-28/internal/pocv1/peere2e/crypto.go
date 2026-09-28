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

package peere2e

import (
	"crypto/ecdh"
	"crypto/hkdf"
	"crypto/rand"
	"crypto/sha256"
	"errors"
	"fmt"

	"golang.org/x/crypto/chacha20poly1305"

	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

const (
	// FrameMagic is the fixed peer_e2e_v1 ciphertext frame prefix.
	FrameMagic = "MP1"
)

const hkdfInfo = "miopunch/poc/v1/controlplane/peer_e2e_v1"

var (
	// ErrInvalidFrame reports a malformed ciphertext frame.
	ErrInvalidFrame = errors.New("invalid peer_e2e_v1 frame")
	// ErrDecryptFailed reports an AEAD authentication failure.
	ErrDecryptFailed = errors.New("peer_e2e_v1 decrypt failed")
)

// SealOptions controls deterministic test fixtures for peer_e2e_v1 sealing.
type SealOptions struct {
	EphemeralPrivateKey []byte
	Nonce               []byte
}

// OpenOptions carries drop-only wire admission hooks for peer_e2e_v1 opening.
type OpenOptions struct {
	NowUnixMs uint64
	SeenMsgID func(msgID string) bool
}

// Frame is the fixed current v1 peer_e2e_v1 ciphertext frame.
type Frame struct {
	EphemeralPublicKey []byte
	Nonce              []byte
	Ciphertext         []byte
}

// Seal signs-then-encrypts a current v1 inner message to one recipient and
// returns the outer header with ciphertext populated.
func Seal(outer wire.OuterHeader, inner wire.InnerMessage, recipientX25519PublicKey []byte, opts SealOptions) (wire.OuterHeader, error) {
	if err := wire.Admit(wire.OpenedMessage{
		Outer: outer,
		Inner: inner,
	}, wire.AdmissionOptions{}); err != nil {
		return wire.OuterHeader{}, err
	}

	recipientPub, err := x25519Curve().NewPublicKey(recipientX25519PublicKey)
	if err != nil {
		return wire.OuterHeader{}, fmt.Errorf("recipient x25519 public key: %w", err)
	}

	ephPriv, err := ephemeralPrivateKey(opts.EphemeralPrivateKey)
	if err != nil {
		return wire.OuterHeader{}, err
	}

	sharedSecret, err := ephPriv.ECDH(recipientPub)
	if err != nil {
		return wire.OuterHeader{}, fmt.Errorf("peer_e2e_v1 ecdh: %w", err)
	}

	aeadKey, err := deriveAEADKey(sharedSecret)
	if err != nil {
		return wire.OuterHeader{}, err
	}

	plaintext, err := inner.MarshalBinary()
	if err != nil {
		return wire.OuterHeader{}, err
	}
	aad, err := wire.BuildOuterAAD(outer)
	if err != nil {
		return wire.OuterHeader{}, err
	}

	nonce, err := sealNonce(opts.Nonce)
	if err != nil {
		return wire.OuterHeader{}, err
	}

	aead, err := chacha20poly1305.NewX(aeadKey)
	if err != nil {
		return wire.OuterHeader{}, fmt.Errorf("peer_e2e_v1 aead: %w", err)
	}

	ciphertext := aead.Seal(nil, nonce, plaintext, aad)
	frame, err := EncodeFrame(Frame{
		EphemeralPublicKey: ephPriv.PublicKey().Bytes(),
		Nonce:              nonce,
		Ciphertext:         ciphertext,
	})
	if err != nil {
		return wire.OuterHeader{}, err
	}

	outer.Ciphertext = frame
	return outer, nil
}

// Open decrypts, verifies, and admits one current v1 peer_e2e_v1 message.
func Open(outer wire.OuterHeader, recipientX25519PrivateKey []byte, opts OpenOptions) (wire.InnerMessage, error) {
	frame, err := DecodeFrame(outer.Ciphertext)
	if err != nil {
		return wire.InnerMessage{}, err
	}

	recipientPriv, err := x25519Curve().NewPrivateKey(recipientX25519PrivateKey)
	if err != nil {
		return wire.InnerMessage{}, fmt.Errorf("recipient x25519 private key: %w", err)
	}
	ephPub, err := x25519Curve().NewPublicKey(frame.EphemeralPublicKey)
	if err != nil {
		return wire.InnerMessage{}, fmt.Errorf("%w: bad ephemeral public key", ErrInvalidFrame)
	}

	sharedSecret, err := recipientPriv.ECDH(ephPub)
	if err != nil {
		return wire.InnerMessage{}, fmt.Errorf("peer_e2e_v1 ecdh: %w", err)
	}
	aeadKey, err := deriveAEADKey(sharedSecret)
	if err != nil {
		return wire.InnerMessage{}, err
	}

	aad, err := wire.BuildOuterAAD(outer)
	if err != nil {
		return wire.InnerMessage{}, err
	}
	aead, err := chacha20poly1305.NewX(aeadKey)
	if err != nil {
		return wire.InnerMessage{}, fmt.Errorf("peer_e2e_v1 aead: %w", err)
	}

	plaintext, err := aead.Open(nil, frame.Nonce, frame.Ciphertext, aad)
	if err != nil {
		return wire.InnerMessage{}, ErrDecryptFailed
	}
	inner, err := wire.UnmarshalInnerMessage(plaintext)
	if err != nil {
		return wire.InnerMessage{}, err
	}

	if err := wire.Admit(wire.OpenedMessage{
		Outer: outer,
		Inner: inner,
	}, wire.AdmissionOptions{
		NowUnixMs: opts.NowUnixMs,
		SeenMsgID: opts.SeenMsgID,
	}); err != nil {
		return wire.InnerMessage{}, err
	}
	return inner, nil
}

// EncodeFrame encodes the fixed peer_e2e_v1 ciphertext frame.
func EncodeFrame(frame Frame) ([]byte, error) {
	if len(frame.EphemeralPublicKey) != 32 {
		return nil, fmt.Errorf("%w: ephemeral public key length %d", ErrInvalidFrame, len(frame.EphemeralPublicKey))
	}
	if len(frame.Nonce) != chacha20poly1305.NonceSizeX {
		return nil, fmt.Errorf("%w: nonce length %d", ErrInvalidFrame, len(frame.Nonce))
	}
	if len(frame.Ciphertext) == 0 {
		return nil, fmt.Errorf("%w: empty ciphertext", ErrInvalidFrame)
	}

	out := make([]byte, 0, len(FrameMagic)+32+chacha20poly1305.NonceSizeX+len(frame.Ciphertext))
	out = append(out, FrameMagic...)
	out = append(out, frame.EphemeralPublicKey...)
	out = append(out, frame.Nonce...)
	out = append(out, frame.Ciphertext...)
	return out, nil
}

// DecodeFrame decodes the fixed peer_e2e_v1 ciphertext frame.
func DecodeFrame(data []byte) (Frame, error) {
	minLen := len(FrameMagic) + 32 + chacha20poly1305.NonceSizeX + 1
	if len(data) < minLen {
		return Frame{}, fmt.Errorf("%w: frame too short", ErrInvalidFrame)
	}
	if string(data[:len(FrameMagic)]) != FrameMagic {
		return Frame{}, fmt.Errorf("%w: bad magic", ErrInvalidFrame)
	}

	offset := len(FrameMagic)
	ephPub := append([]byte(nil), data[offset:offset+32]...)
	offset += 32
	nonce := append([]byte(nil), data[offset:offset+chacha20poly1305.NonceSizeX]...)
	offset += chacha20poly1305.NonceSizeX
	ciphertext := append([]byte(nil), data[offset:]...)
	if len(ciphertext) == 0 {
		return Frame{}, fmt.Errorf("%w: empty ciphertext", ErrInvalidFrame)
	}

	return Frame{
		EphemeralPublicKey: ephPub,
		Nonce:              nonce,
		Ciphertext:         ciphertext,
	}, nil
}

func deriveAEADKey(sharedSecret []byte) ([]byte, error) {
	key, err := hkdf.Key(sha256.New, sharedSecret, nil, hkdfInfo, chacha20poly1305.KeySize)
	if err != nil {
		return nil, fmt.Errorf("peer_e2e_v1 hkdf: %w", err)
	}
	return key, nil
}

func ephemeralPrivateKey(raw []byte) (*ecdh.PrivateKey, error) {
	if len(raw) != 0 {
		priv, err := x25519Curve().NewPrivateKey(raw)
		if err != nil {
			return nil, fmt.Errorf("peer_e2e_v1 ephemeral private key: %w", err)
		}
		return priv, nil
	}

	priv, err := x25519Curve().GenerateKey(rand.Reader)
	if err != nil {
		return nil, fmt.Errorf("peer_e2e_v1 ephemeral private key: %w", err)
	}
	return priv, nil
}

func sealNonce(raw []byte) ([]byte, error) {
	if len(raw) != 0 {
		if len(raw) != chacha20poly1305.NonceSizeX {
			return nil, fmt.Errorf("%w: nonce length %d", ErrInvalidFrame, len(raw))
		}
		return append([]byte(nil), raw...), nil
	}

	nonce := make([]byte, chacha20poly1305.NonceSizeX)
	if _, err := rand.Read(nonce); err != nil {
		return nil, fmt.Errorf("peer_e2e_v1 nonce: %w", err)
	}
	return nonce, nil
}

func x25519Curve() ecdh.Curve {
	return ecdh.X25519()
}
