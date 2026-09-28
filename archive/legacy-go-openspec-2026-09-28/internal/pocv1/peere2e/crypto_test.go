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
	"bytes"
	"crypto/ecdh"
	"crypto/ed25519"
	"encoding/hex"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

func TestSealAndOpenMatchGolden(t *testing.T) {
	t.Parallel()

	fx := mustCryptoFixture(t)
	sealedOuter, err := Seal(fx.outer, fx.inner, fx.recipientX25519Pub, SealOptions{
		EphemeralPrivateKey: fx.ephemeralX25519Priv,
		Nonce:               fx.nonce,
	})
	if err != nil {
		t.Fatalf("Seal() error = %v", err)
	}
	assertGoldenHex(t, "../wire/testdata/ciphertext.hex", sealedOuter.Ciphertext)

	opened, err := Open(sealedOuter, fx.recipientX25519Priv, OpenOptions{})
	if err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	if diff := diffInner(fx.inner, opened); diff != "" {
		t.Fatalf("Open() inner mismatch (-want +got):\n%s", diff)
	}
}

func TestOpenRejectsCiphertextAndAADTampering(t *testing.T) {
	t.Parallel()

	fx := mustCryptoFixture(t)
	sealedOuter, err := Seal(fx.outer, fx.inner, fx.recipientX25519Pub, SealOptions{
		EphemeralPrivateKey: fx.ephemeralX25519Priv,
		Nonce:               fx.nonce,
	})
	if err != nil {
		t.Fatalf("Seal() error = %v", err)
	}

	tamperedCiphertext := sealedOuter
	tamperedCiphertext.Ciphertext = append([]byte(nil), sealedOuter.Ciphertext...)
	tamperedCiphertext.Ciphertext[len(tamperedCiphertext.Ciphertext)-1] ^= 0x01
	if _, err := Open(tamperedCiphertext, fx.recipientX25519Priv, OpenOptions{}); !errors.Is(err, ErrDecryptFailed) {
		t.Fatalf("Open(ciphertext tamper) error = %v, want %v", err, ErrDecryptFailed)
	}

	tamperedAAD := sealedOuter
	tamperedAAD.MsgID = fx.altMsgID
	if _, err := Open(tamperedAAD, fx.recipientX25519Priv, OpenOptions{}); !errors.Is(err, ErrDecryptFailed) {
		t.Fatalf("Open(aad tamper) error = %v, want %v", err, ErrDecryptFailed)
	}
}

func TestOpenAllowsOuterSrcMismatch(t *testing.T) {
	t.Parallel()

	fx := mustCryptoFixture(t)
	sealedOuter, err := Seal(fx.outer, fx.inner, fx.recipientX25519Pub, SealOptions{
		EphemeralPrivateKey: fx.ephemeralX25519Priv,
		Nonce:               fx.nonce,
	})
	if err != nil {
		t.Fatalf("Seal() error = %v", err)
	}

	sealedOuter.SrcPeerID = fx.altSrcPeerID
	opened, err := Open(sealedOuter, fx.recipientX25519Priv, OpenOptions{})
	if err != nil {
		t.Fatalf("Open(src mismatch) error = %v", err)
	}
	if opened.SenderPeerID != fx.inner.SenderPeerID {
		t.Fatalf("Open(src mismatch) sender_peer_id = %q, want %q", opened.SenderPeerID, fx.inner.SenderPeerID)
	}
}

func TestOpenRejectsReplayAndExpiry(t *testing.T) {
	t.Parallel()

	fx := mustCryptoFixture(t)
	sealedOuter, err := Seal(fx.outer, fx.inner, fx.recipientX25519Pub, SealOptions{
		EphemeralPrivateKey: fx.ephemeralX25519Priv,
		Nonce:               fx.nonce,
	})
	if err != nil {
		t.Fatalf("Seal() error = %v", err)
	}

	if _, err := Open(sealedOuter, fx.recipientX25519Priv, OpenOptions{
		NowUnixMs: fx.inner.ExpiresAtUnixMs + 1,
	}); !errors.Is(err, wire.ErrExpired) {
		t.Fatalf("Open(expired) error = %v, want %v", err, wire.ErrExpired)
	}

	if _, err := Open(sealedOuter, fx.recipientX25519Priv, OpenOptions{
		SeenMsgID: func(msgID string) bool { return msgID == fx.inner.MsgID },
	}); !errors.Is(err, wire.ErrReplay) {
		t.Fatalf("Open(replay) error = %v, want %v", err, wire.ErrReplay)
	}
}

type cryptoFixture struct {
	outer               wire.OuterHeader
	inner               wire.InnerMessage
	recipientX25519Priv []byte
	recipientX25519Pub  []byte
	ephemeralX25519Priv []byte
	nonce               []byte
	altMsgID            string
	altSrcPeerID        string
}

func mustCryptoFixture(t *testing.T) cryptoFixture {
	t.Helper()

	senderSeed := bytes.Repeat([]byte{0x11}, ed25519.SeedSize)
	senderPriv := ed25519.NewKeyFromSeed(senderSeed)
	senderPub := senderPriv.Public().(ed25519.PublicKey)
	senderPeerID, err := wire.PeerIDFromEd25519Pub(senderPub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(sender) error = %v", err)
	}

	recipientSeed := bytes.Repeat([]byte{0x22}, ed25519.SeedSize)
	recipientPriv := ed25519.NewKeyFromSeed(recipientSeed)
	recipientPub := recipientPriv.Public().(ed25519.PublicKey)
	recipientPeerID, err := wire.PeerIDFromEd25519Pub(recipientPub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(recipient) error = %v", err)
	}

	msgID, err := wire.CanonicalizeMsgID("JBSWY3DPEHPK3PXPJBSWY3DPAA")
	if err != nil {
		t.Fatalf("CanonicalizeMsgID() error = %v", err)
	}
	altMsgID, err := wire.CanonicalizeMsgID("MFRGGZDFMZTWQ2LKNNWG23TPOI")
	if err != nil {
		t.Fatalf("CanonicalizeMsgID(alt) error = %v", err)
	}

	inner := wire.InnerMessage{
		DstPeerID:       recipientPeerID,
		MsgID:           msgID,
		CreatedAtUnixMs: 1_717_000_000_000,
		ExpiresAtUnixMs: 1_717_000_030_000,
		SenderPeerID:    senderPeerID,
		SenderEd25519:   append([]byte(nil), senderPub...),
		Kind:            wire.KindJoinRequest,
		Body:            []byte(`{"invite_id":"INV-01","reply_topic":"mp/v1/reply/demo"}`),
	}
	if err := wire.SignInner(senderPriv, &inner); err != nil {
		t.Fatalf("SignInner() error = %v", err)
	}

	outer := wire.OuterHeader{
		Version:         wire.OuterVersionV1,
		DstPeerID:       recipientPeerID,
		SrcPeerID:       senderPeerID,
		MsgID:           msgID,
		ExpiresAtUnixMs: inner.ExpiresAtUnixMs,
		Scheme:          wire.SchemePeerE2EV1,
	}

	recipientX25519Priv := bytes.Repeat([]byte{0x33}, 32)
	recipientX25519Pub, err := x25519Public(recipientX25519Priv)
	if err != nil {
		t.Fatalf("x25519Public(recipient) error = %v", err)
	}
	ephemeralX25519Priv := bytes.Repeat([]byte{0x44}, 32)

	otherSeed := bytes.Repeat([]byte{0x66}, ed25519.SeedSize)
	otherPriv := ed25519.NewKeyFromSeed(otherSeed)
	otherPub := otherPriv.Public().(ed25519.PublicKey)
	altSrcPeerID, err := wire.PeerIDFromEd25519Pub(otherPub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(other) error = %v", err)
	}

	return cryptoFixture{
		outer:               outer,
		inner:               inner,
		recipientX25519Priv: recipientX25519Priv,
		recipientX25519Pub:  recipientX25519Pub,
		ephemeralX25519Priv: ephemeralX25519Priv,
		nonce:               bytes.Repeat([]byte{0x55}, 24),
		altMsgID:            altMsgID,
		altSrcPeerID:        altSrcPeerID,
	}
}

func x25519Public(rawPrivate []byte) ([]byte, error) {
	priv, err := ecdh.X25519().NewPrivateKey(rawPrivate)
	if err != nil {
		return nil, err
	}
	return priv.PublicKey().Bytes(), nil
}

func diffInner(want, got wire.InnerMessage) string {
	switch {
	case want.DstPeerID != got.DstPeerID:
		return "dst_peer_id mismatch"
	case want.MsgID != got.MsgID:
		return "msg_id mismatch"
	case want.CreatedAtUnixMs != got.CreatedAtUnixMs:
		return "created_at_unix_ms mismatch"
	case want.ExpiresAtUnixMs != got.ExpiresAtUnixMs:
		return "expires_at_unix_ms mismatch"
	case want.SenderPeerID != got.SenderPeerID:
		return "sender_peer_id mismatch"
	case !bytes.Equal(want.SenderEd25519, got.SenderEd25519):
		return "sender_ed25519 mismatch"
	case want.Kind != got.Kind:
		return "kind mismatch"
	case want.InReplyTo != got.InReplyTo:
		return "in_reply_to mismatch"
	case !bytes.Equal(want.Body, got.Body):
		return "body mismatch"
	case !bytes.Equal(want.Signature, got.Signature):
		return "signature mismatch"
	default:
		return ""
	}
}

func assertGoldenHex(t *testing.T, relativePath string, got []byte) {
	t.Helper()

	data, err := os.ReadFile(filepath.Join(relativePath))
	if err != nil {
		t.Fatalf("read golden %s: %v\nwrite this hex:\n%s", relativePath, err, hex.EncodeToString(got))
	}

	want, err := hex.DecodeString(strings.TrimSpace(string(data)))
	if err != nil {
		t.Fatalf("decode golden %s: %v", relativePath, err)
	}
	if !bytes.Equal(got, want) {
		t.Fatalf("%s mismatch\n got: %s\nwant: %s", relativePath, hex.EncodeToString(got), hex.EncodeToString(want))
	}
}
