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
	"fmt"
	"strings"
)

const (
	// KindJoinRequest is the current v1 enroll bootstrap request kind.
	KindJoinRequest = "join_request"
	// KindEnrollResponse is the current v1 enroll bootstrap response kind.
	KindEnrollResponse = "enroll_response"
	// KindDialOffer is the current v1 punch offer kind.
	KindDialOffer = "dial_offer"
	// KindDialAnswer is the current v1 punch answer kind.
	KindDialAnswer = "dial_answer"
)

const transcriptDomainV1 = "miopunch/poc/v1/controlplane/transcript/peer_e2e_v1"

const (
	innerTagDstPeerID       = 1
	innerTagMsgID           = 2
	innerTagCreatedAtUnixMs = 3
	innerTagExpiresAtUnixMs = 4
	innerTagSenderPeerID    = 5
	innerTagSenderEd25519   = 6
	innerTagKind            = 7
	innerTagInReplyTo       = 8
	innerTagBody            = 9
	innerTagSignature       = 10
)

var innerAllowedTags = map[uint64]struct{}{
	innerTagDstPeerID:       {},
	innerTagMsgID:           {},
	innerTagCreatedAtUnixMs: {},
	innerTagExpiresAtUnixMs: {},
	innerTagSenderPeerID:    {},
	innerTagSenderEd25519:   {},
	innerTagKind:            {},
	innerTagInReplyTo:       {},
	innerTagBody:            {},
	innerTagSignature:       {},
}

// InnerMessage is the signed current v1 peer-targeted message body.
type InnerMessage struct {
	DstPeerID       string
	MsgID           string
	CreatedAtUnixMs uint64
	ExpiresAtUnixMs uint64
	SenderPeerID    string
	SenderEd25519   []byte
	Kind            string
	InReplyTo       string
	Body            []byte
	Signature       []byte
}

// MarshalBinary encodes the inner message as canonical TLV.
func (m InnerMessage) MarshalBinary() ([]byte, error) {
	normalized, err := normalizeInner(m, true)
	if err != nil {
		return nil, err
	}
	return marshalInner(normalized, true)
}

// UnmarshalInnerMessage decodes one signed inner message from canonical TLV.
func UnmarshalInnerMessage(data []byte) (InnerMessage, error) {
	fields, err := decodeFieldsStrict(data, cloneAllowed(innerAllowedTags))
	if err != nil {
		return InnerMessage{}, err
	}
	index := indexFields(fields)

	requireASCII := func(tag uint64, name string) (string, error) {
		field, ok := index[tag]
		if !ok {
			return "", fmt.Errorf("%w: missing %s", ErrInvalidFieldValue, name)
		}
		return DecodeASCIIField(field)
	}
	requireU64 := func(tag uint64, name string) (uint64, error) {
		field, ok := index[tag]
		if !ok {
			return 0, fmt.Errorf("%w: missing %s", ErrInvalidFieldValue, name)
		}
		return DecodeU64Field(field)
	}
	requireBytes := func(tag uint64, name string) ([]byte, error) {
		field, ok := index[tag]
		if !ok {
			return nil, fmt.Errorf("%w: missing %s", ErrInvalidFieldValue, name)
		}
		return append([]byte(nil), field.Value...), nil
	}

	dstPeerID, err := requireASCII(innerTagDstPeerID, "dst_peer_id")
	if err != nil {
		return InnerMessage{}, err
	}
	msgID, err := requireASCII(innerTagMsgID, "msg_id")
	if err != nil {
		return InnerMessage{}, err
	}
	createdAtUnixMs, err := requireU64(innerTagCreatedAtUnixMs, "created_at")
	if err != nil {
		return InnerMessage{}, err
	}
	expiresAtUnixMs, err := requireU64(innerTagExpiresAtUnixMs, "expires_at")
	if err != nil {
		return InnerMessage{}, err
	}
	senderPeerID, err := requireASCII(innerTagSenderPeerID, "sender_peer_id")
	if err != nil {
		return InnerMessage{}, err
	}
	senderEd25519, err := requireBytes(innerTagSenderEd25519, "sender_ed25519")
	if err != nil {
		return InnerMessage{}, err
	}
	kind, err := requireASCII(innerTagKind, "kind")
	if err != nil {
		return InnerMessage{}, err
	}
	body, err := requireBytes(innerTagBody, "body")
	if err != nil {
		return InnerMessage{}, err
	}
	signature, err := requireBytes(innerTagSignature, "signature")
	if err != nil {
		return InnerMessage{}, err
	}

	inReplyTo := ""
	if field, ok := index[innerTagInReplyTo]; ok {
		inReplyTo, err = DecodeASCIIField(field)
		if err != nil {
			return InnerMessage{}, err
		}
	}

	return normalizeInner(InnerMessage{
		DstPeerID:       dstPeerID,
		MsgID:           msgID,
		CreatedAtUnixMs: createdAtUnixMs,
		ExpiresAtUnixMs: expiresAtUnixMs,
		SenderPeerID:    senderPeerID,
		SenderEd25519:   senderEd25519,
		Kind:            kind,
		InReplyTo:       inReplyTo,
		Body:            body,
		Signature:       signature,
	}, true)
}

// BuildTranscript returns the fixed current v1 Ed25519 transcript bytes.
func BuildTranscript(m InnerMessage) ([]byte, error) {
	normalized, err := normalizeInner(m, false)
	if err != nil {
		return nil, err
	}

	body, err := marshalInner(normalized, false)
	if err != nil {
		return nil, err
	}

	out := make([]byte, 0, len(transcriptDomainV1)+len(body)+1)
	out = append(out, transcriptDomainV1...)
	out = append(out, 0x00)
	out = append(out, body...)
	return out, nil
}

// SignInner signs the current v1 transcript directly with Ed25519.
func SignInner(priv ed25519.PrivateKey, m *InnerMessage) error {
	if m == nil {
		return fmt.Errorf("%w: nil inner message", ErrInvalidFieldValue)
	}
	if len(priv) != ed25519.PrivateKeySize {
		return fmt.Errorf("%w: invalid ed25519 private key length: %d", ErrInvalidFieldValue, len(priv))
	}

	pub, ok := priv.Public().(ed25519.PublicKey)
	if !ok {
		return fmt.Errorf("%w: ed25519 public key unavailable", ErrInvalidFieldValue)
	}
	if len(m.SenderEd25519) == 0 {
		m.SenderEd25519 = append([]byte(nil), pub...)
	}
	if !bytes.Equal(m.SenderEd25519, pub) {
		return fmt.Errorf("%w: sender_ed25519 does not match signer", ErrInvalidFieldValue)
	}

	peerID, err := PeerIDFromEd25519Pub(pub)
	if err != nil {
		return fmt.Errorf("derive sender_peer_id: %w", err)
	}
	if m.SenderPeerID == "" {
		m.SenderPeerID = peerID
	}

	transcript, err := BuildTranscript(*m)
	if err != nil {
		return fmt.Errorf("build transcript: %w", err)
	}

	m.Signature = ed25519.Sign(priv, transcript)
	return nil
}

// VerifyInner verifies the signed current v1 transcript and field invariants.
func VerifyInner(m InnerMessage) error {
	normalized, err := normalizeInner(m, true)
	if err != nil {
		return fmt.Errorf("normalize inner message: %w", err)
	}

	transcript, err := BuildTranscript(normalized)
	if err != nil {
		return fmt.Errorf("build transcript: %w", err)
	}

	if !ed25519.Verify(normalized.SenderEd25519, transcript, normalized.Signature) {
		return ErrInvalidSignature
	}
	return nil
}

// IsSupportedKind reports whether kind is within the current v1 allowlist.
func IsSupportedKind(kind string) bool {
	switch kind {
	case KindJoinRequest, KindEnrollResponse, KindDialOffer, KindDialAnswer:
		return true
	default:
		return false
	}
}

func marshalInner(m InnerMessage, includeSignature bool) ([]byte, error) {
	out := make([]byte, 0, len(m.Body)+len(m.Signature)+160)
	var err error

	out, err = AppendASCIIField(out, innerTagDstPeerID, m.DstPeerID)
	if err != nil {
		return nil, err
	}
	out, err = AppendASCIIField(out, innerTagMsgID, m.MsgID)
	if err != nil {
		return nil, err
	}
	out = AppendU64Field(out, innerTagCreatedAtUnixMs, m.CreatedAtUnixMs)
	out = AppendU64Field(out, innerTagExpiresAtUnixMs, m.ExpiresAtUnixMs)
	out, err = AppendASCIIField(out, innerTagSenderPeerID, m.SenderPeerID)
	if err != nil {
		return nil, err
	}
	out = AppendBytesField(out, innerTagSenderEd25519, m.SenderEd25519)
	out, err = AppendASCIIField(out, innerTagKind, m.Kind)
	if err != nil {
		return nil, err
	}
	if m.InReplyTo != "" {
		out, err = AppendASCIIField(out, innerTagInReplyTo, m.InReplyTo)
		if err != nil {
			return nil, err
		}
	}
	out = AppendBytesField(out, innerTagBody, m.Body)
	if includeSignature {
		out = AppendBytesField(out, innerTagSignature, m.Signature)
	}
	return out, nil
}

func normalizeInner(m InnerMessage, requireSignature bool) (InnerMessage, error) {
	dstPeerID, err := CanonicalizePeerID(m.DstPeerID)
	if err != nil {
		return InnerMessage{}, err
	}
	msgID, err := CanonicalizeMsgID(m.MsgID)
	if err != nil {
		return InnerMessage{}, err
	}
	if m.CreatedAtUnixMs == 0 {
		return InnerMessage{}, fmt.Errorf("%w: missing created_at", ErrInvalidFieldValue)
	}
	if m.ExpiresAtUnixMs == 0 {
		return InnerMessage{}, fmt.Errorf("%w: missing expires_at", ErrInvalidFieldValue)
	}
	if m.CreatedAtUnixMs > m.ExpiresAtUnixMs {
		return InnerMessage{}, fmt.Errorf("%w: created_at > expires_at", ErrInvalidFieldValue)
	}

	if len(m.SenderEd25519) != ed25519.PublicKeySize {
		return InnerMessage{}, fmt.Errorf("%w: invalid sender_ed25519 length: %d", ErrInvalidFieldValue, len(m.SenderEd25519))
	}
	senderPeerID, err := CanonicalizePeerID(m.SenderPeerID)
	if err != nil {
		return InnerMessage{}, err
	}
	derivedPeerID, err := PeerIDFromEd25519Pub(ed25519.PublicKey(m.SenderEd25519))
	if err != nil {
		return InnerMessage{}, err
	}
	if senderPeerID != derivedPeerID {
		return InnerMessage{}, fmt.Errorf("%w: sender_peer_id does not match sender_ed25519", ErrInvalidFieldValue)
	}

	kind := strings.TrimSpace(m.Kind)
	if !IsSupportedKind(kind) {
		return InnerMessage{}, fmt.Errorf("%w: %q", ErrUnsupportedKind, m.Kind)
	}

	inReplyTo := ""
	if m.InReplyTo != "" {
		inReplyTo, err = CanonicalizeMsgID(m.InReplyTo)
		if err != nil {
			return InnerMessage{}, err
		}
	}

	signature := append([]byte(nil), m.Signature...)
	if requireSignature {
		if len(signature) != ed25519.SignatureSize {
			return InnerMessage{}, fmt.Errorf("%w: invalid signature length: %d", ErrInvalidFieldValue, len(signature))
		}
	}
	if !requireSignature {
		signature = nil
	}

	return InnerMessage{
		DstPeerID:       dstPeerID,
		MsgID:           msgID,
		CreatedAtUnixMs: m.CreatedAtUnixMs,
		ExpiresAtUnixMs: m.ExpiresAtUnixMs,
		SenderPeerID:    senderPeerID,
		SenderEd25519:   append([]byte(nil), m.SenderEd25519...),
		Kind:            kind,
		InReplyTo:       inReplyTo,
		Body:            append([]byte(nil), m.Body...),
		Signature:       signature,
	}, nil
}
