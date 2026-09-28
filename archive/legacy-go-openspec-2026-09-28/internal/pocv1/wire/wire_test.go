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

package wire

import (
	"bytes"
	"crypto/ed25519"
	"encoding/hex"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestDecodeFieldsStrictRejectsInvalidInputs(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name    string
		data    []byte
		wantErr error
	}{
		{
			name:    "unknown tag",
			data:    AppendBytesField(nil, 99, []byte("x")),
			wantErr: ErrUnknownTag,
		},
		{
			name: "duplicate tag",
			data: append(
				AppendBytesField(nil, 1, []byte("a")),
				AppendBytesField(nil, 1, []byte("b"))...,
			),
			wantErr: ErrDuplicateTag,
		},
		{
			name: "non canonical tag uvarint",
			data: []byte{
				0x81, 0x00,
				0x01,
				'x',
			},
			wantErr: ErrNonCanonicalUvarint,
		},
		{
			name: "out of order",
			data: append(
				AppendBytesField(nil, 2, []byte("b")),
				AppendBytesField(nil, 1, []byte("a"))...,
			),
			wantErr: ErrOutOfOrderField,
		},
		{
			name: "truncated length",
			data: []byte{
				0x01,
				0x03,
				'a',
			},
			wantErr: ErrTruncatedTLV,
		},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			_, err := DecodeFieldsStrict(tt.data, 1, 2, 3)
			if !errors.Is(err, tt.wantErr) {
				t.Fatalf("DecodeFieldsStrict(%x) error = %v, want %v", tt.data, err, tt.wantErr)
			}
		})
	}
}

func TestAppendASCIIFieldRejectsNonASCII(t *testing.T) {
	t.Parallel()

	_, err := AppendASCIIField(nil, 1, "hi-\u00ff")
	if !errors.Is(err, ErrInvalidASCII) {
		t.Fatalf("AppendASCIIField(nonASCII) error = %v, want %v", err, ErrInvalidASCII)
	}
}

func TestNetworkIDRoundTripAndRejectsInvalidInputs(t *testing.T) {
	t.Parallel()

	raw := bytes.Repeat([]byte{0x5a}, RawIDLen)
	encoded, err := EncodeNetworkID(raw)
	if err != nil {
		t.Fatalf("EncodeNetworkID() error = %v, want nil", err)
	}
	if len(encoded) != CanonicalIDLen {
		t.Fatalf("EncodeNetworkID() length = %d, want %d", len(encoded), CanonicalIDLen)
	}

	decoded, err := DecodeNetworkID(encoded)
	if err != nil {
		t.Fatalf("DecodeNetworkID(%q) error = %v, want nil", encoded, err)
	}
	if !bytes.Equal(decoded, raw) {
		t.Fatalf("DecodeNetworkID(%q) = %x, want %x", encoded, decoded, raw)
	}

	reencoded, err := EncodeNetworkID(decoded)
	if err != nil {
		t.Fatalf("EncodeNetworkID(roundtrip) error = %v, want nil", err)
	}
	if reencoded != encoded {
		t.Fatalf("EncodeNetworkID(roundtrip) = %q, want %q", reencoded, encoded)
	}

	tests := []struct {
		name string
		run  func() error
	}{
		{
			name: "encode wrong raw length",
			run: func() error {
				_, err := EncodeNetworkID(raw[:RawIDLen-1])
				return err
			},
		},
		{
			name: "decode wrong length",
			run: func() error {
				_, err := DecodeNetworkID(encoded[:CanonicalIDLen-1])
				return err
			},
		},
		{
			name: "decode invalid character",
			run: func() error {
				bad := encoded[:CanonicalIDLen-1] + "!"
				_, err := DecodeNetworkID(bad)
				return err
			},
		},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			if err := tt.run(); err == nil {
				t.Fatalf("%s error = nil, want non-nil", tt.name)
			}
		})
	}
}

func TestBuildTranscriptAndEncodingsMatchGolden(t *testing.T) {
	t.Parallel()

	fx := mustFixture(t)

	outerBinary, err := fx.outer.MarshalBinary()
	if err != nil {
		t.Fatalf("OuterHeader.MarshalBinary() error = %v", err)
	}
	assertGoldenHex(t, "testdata/outer.hex", outerBinary)

	innerBinary, err := fx.inner.MarshalBinary()
	if err != nil {
		t.Fatalf("InnerMessage.MarshalBinary() error = %v", err)
	}
	assertGoldenHex(t, "testdata/inner.hex", innerBinary)

	transcript, err := BuildTranscript(fx.inner)
	if err != nil {
		t.Fatalf("BuildTranscript() error = %v", err)
	}
	assertGoldenHex(t, "testdata/transcript.hex", transcript)
}

func TestVerifyInnerRejectsTamperedBody(t *testing.T) {
	t.Parallel()

	fx := mustFixture(t)
	fx.inner.Body = append([]byte(nil), fx.inner.Body...)
	fx.inner.Body[0] ^= 0x01

	err := VerifyInner(fx.inner)
	if !errors.Is(err, ErrInvalidSignature) {
		t.Fatalf("VerifyInner(tampered) error = %v, want %v", err, ErrInvalidSignature)
	}
}

func TestAdmitRejectsMismatchExpiryReplayAndExpiry(t *testing.T) {
	t.Parallel()

	fx := mustFixture(t)

	mismatch := OpenedMessage{
		Outer: fx.outer,
		Inner: fx.inner,
	}
	mismatch.Outer.ExpiresAtUnixMs++
	if err := Admit(mismatch, AdmissionOptions{}); !errors.Is(err, ErrOuterInnerMismatch) {
		t.Fatalf("Admit(expiry mismatch) error = %v, want %v", err, ErrOuterInnerMismatch)
	}

	expired := OpenedMessage{
		Outer: fx.outer,
		Inner: fx.inner,
	}
	if err := Admit(expired, AdmissionOptions{NowUnixMs: fx.inner.ExpiresAtUnixMs + 1}); !errors.Is(err, ErrExpired) {
		t.Fatalf("Admit(expired) error = %v, want %v", err, ErrExpired)
	}

	replayed := OpenedMessage{
		Outer: fx.outer,
		Inner: fx.inner,
	}
	if err := Admit(replayed, AdmissionOptions{SeenMsgID: func(msgID string) bool { return msgID == fx.inner.MsgID }}); !errors.Is(err, ErrReplay) {
		t.Fatalf("Admit(replay) error = %v, want %v", err, ErrReplay)
	}
}

type fixture struct {
	senderSeed      []byte
	senderPriv      ed25519.PrivateKey
	senderPub       ed25519.PublicKey
	senderPeerID    string
	recipientSeed   []byte
	recipientPriv   ed25519.PrivateKey
	recipientPub    ed25519.PublicKey
	recipientPeerID string
	msgID           string
	outer           OuterHeader
	inner           InnerMessage
}

func mustFixture(t *testing.T) fixture {
	t.Helper()

	senderSeed := bytes.Repeat([]byte{0x11}, ed25519.SeedSize)
	senderPriv := ed25519.NewKeyFromSeed(senderSeed)
	senderPub := senderPriv.Public().(ed25519.PublicKey)
	senderPeerID, err := PeerIDFromEd25519Pub(senderPub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(sender) error = %v", err)
	}

	recipientSeed := bytes.Repeat([]byte{0x22}, ed25519.SeedSize)
	recipientPriv := ed25519.NewKeyFromSeed(recipientSeed)
	recipientPub := recipientPriv.Public().(ed25519.PublicKey)
	recipientPeerID, err := PeerIDFromEd25519Pub(recipientPub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(recipient) error = %v", err)
	}

	msgID, err := CanonicalizeMsgID("JBSWY3DPEHPK3PXPJBSWY3DPAA")
	if err != nil {
		t.Fatalf("CanonicalizeMsgID() error = %v", err)
	}

	inner := InnerMessage{
		DstPeerID:       recipientPeerID,
		MsgID:           msgID,
		CreatedAtUnixMs: 1_717_000_000_000,
		ExpiresAtUnixMs: 1_717_000_030_000,
		SenderPeerID:    senderPeerID,
		SenderEd25519:   append([]byte(nil), senderPub...),
		Kind:            KindJoinRequest,
		Body:            []byte(`{"invite_id":"INV-01","reply_topic":"mp/v1/reply/demo"}`),
	}
	if err := SignInner(senderPriv, &inner); err != nil {
		t.Fatalf("SignInner() error = %v", err)
	}

	outer := OuterHeader{
		Version:         OuterVersionV1,
		DstPeerID:       recipientPeerID,
		SrcPeerID:       senderPeerID,
		MsgID:           msgID,
		ExpiresAtUnixMs: inner.ExpiresAtUnixMs,
		Scheme:          SchemePeerE2EV1,
		Ciphertext:      []byte("ciphertext-fixture"),
	}

	return fixture{
		senderSeed:      senderSeed,
		senderPriv:      senderPriv,
		senderPub:       senderPub,
		senderPeerID:    senderPeerID,
		recipientSeed:   recipientSeed,
		recipientPriv:   recipientPriv,
		recipientPub:    recipientPub,
		recipientPeerID: recipientPeerID,
		msgID:           msgID,
		outer:           outer,
		inner:           inner,
	}
}

func assertGoldenHex(t *testing.T, relativePath string, got []byte) {
	t.Helper()

	path := filepath.Join(relativePath)
	data, err := os.ReadFile(path)
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
