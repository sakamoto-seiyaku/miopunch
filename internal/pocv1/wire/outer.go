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

import "fmt"

const (
	// OuterVersionV1 is the only supported current v1 outer header version.
	OuterVersionV1 = 1
	// SchemePeerE2EV1 is the only supported current v1 peer-targeted scheme.
	SchemePeerE2EV1 = "peer_e2e_v1"
)

const (
	outerTagVersion    = 1
	outerTagDstPeerID  = 2
	outerTagSrcPeerID  = 3
	outerTagMsgID      = 4
	outerTagExpiresAt  = 5
	outerTagScheme     = 6
	outerTagCiphertext = 7
)

var outerAllowedTags = map[uint64]struct{}{
	outerTagVersion:    {},
	outerTagDstPeerID:  {},
	outerTagSrcPeerID:  {},
	outerTagMsgID:      {},
	outerTagExpiresAt:  {},
	outerTagScheme:     {},
	outerTagCiphertext: {},
}

// OuterHeader is the plaintext relay header for a current v1 peer-targeted
// message.
type OuterHeader struct {
	Version         uint64
	DstPeerID       string
	SrcPeerID       string
	MsgID           string
	ExpiresAtUnixMs uint64
	Scheme          string
	Ciphertext      []byte
}

// MarshalBinary encodes the outer header as canonical TLV.
func (o OuterHeader) MarshalBinary() ([]byte, error) {
	normalized, err := normalizeOuter(o)
	if err != nil {
		return nil, err
	}
	if len(normalized.Ciphertext) == 0 {
		return nil, fmt.Errorf("%w: missing ciphertext", ErrInvalidFieldValue)
	}

	out := make([]byte, 0, len(normalized.Ciphertext)+96)
	out = AppendU64Field(out, outerTagVersion, normalized.Version)
	out, err = AppendASCIIField(out, outerTagDstPeerID, normalized.DstPeerID)
	if err != nil {
		return nil, err
	}
	if normalized.SrcPeerID != "" {
		out, err = AppendASCIIField(out, outerTagSrcPeerID, normalized.SrcPeerID)
		if err != nil {
			return nil, err
		}
	}
	out, err = AppendASCIIField(out, outerTagMsgID, normalized.MsgID)
	if err != nil {
		return nil, err
	}
	out = AppendU64Field(out, outerTagExpiresAt, normalized.ExpiresAtUnixMs)
	out, err = AppendASCIIField(out, outerTagScheme, normalized.Scheme)
	if err != nil {
		return nil, err
	}
	out = AppendBytesField(out, outerTagCiphertext, normalized.Ciphertext)
	return out, nil
}

// UnmarshalOuterHeader decodes one outer header from canonical TLV.
func UnmarshalOuterHeader(data []byte) (OuterHeader, error) {
	fields, err := decodeFieldsStrict(data, cloneAllowed(outerAllowedTags))
	if err != nil {
		return OuterHeader{}, err
	}
	index := indexFields(fields)

	versionField, ok := index[outerTagVersion]
	if !ok {
		return OuterHeader{}, fmt.Errorf("%w: missing version", ErrInvalidFieldValue)
	}
	version, err := DecodeU64Field(versionField)
	if err != nil {
		return OuterHeader{}, err
	}

	dstField, ok := index[outerTagDstPeerID]
	if !ok {
		return OuterHeader{}, fmt.Errorf("%w: missing dst", ErrInvalidFieldValue)
	}
	dstPeerID, err := DecodeASCIIField(dstField)
	if err != nil {
		return OuterHeader{}, err
	}

	msgField, ok := index[outerTagMsgID]
	if !ok {
		return OuterHeader{}, fmt.Errorf("%w: missing msg_id", ErrInvalidFieldValue)
	}
	msgID, err := DecodeASCIIField(msgField)
	if err != nil {
		return OuterHeader{}, err
	}

	expiresField, ok := index[outerTagExpiresAt]
	if !ok {
		return OuterHeader{}, fmt.Errorf("%w: missing expires_at", ErrInvalidFieldValue)
	}
	expiresAtUnixMs, err := DecodeU64Field(expiresField)
	if err != nil {
		return OuterHeader{}, err
	}

	schemeField, ok := index[outerTagScheme]
	if !ok {
		return OuterHeader{}, fmt.Errorf("%w: missing scheme", ErrInvalidFieldValue)
	}
	scheme, err := DecodeASCIIField(schemeField)
	if err != nil {
		return OuterHeader{}, err
	}

	ciphertextField, ok := index[outerTagCiphertext]
	if !ok {
		return OuterHeader{}, fmt.Errorf("%w: missing ciphertext", ErrInvalidFieldValue)
	}

	srcPeerID := ""
	if srcField, ok := index[outerTagSrcPeerID]; ok {
		srcPeerID, err = DecodeASCIIField(srcField)
		if err != nil {
			return OuterHeader{}, err
		}
	}

	return normalizeOuter(OuterHeader{
		Version:         version,
		DstPeerID:       dstPeerID,
		SrcPeerID:       srcPeerID,
		MsgID:           msgID,
		ExpiresAtUnixMs: expiresAtUnixMs,
		Scheme:          scheme,
		Ciphertext:      append([]byte(nil), ciphertextField.Value...),
	})
}

// Validate checks the outer fields other than ciphertext presence.
func (o OuterHeader) Validate() error {
	_, err := normalizeOuter(o)
	return err
}

// OuterForInner derives the current v1 outer header fields owned by the inner
// message contract.
func OuterForInner(inner InnerMessage) (OuterHeader, error) {
	normalized, err := normalizeInner(inner, true)
	if err != nil {
		return OuterHeader{}, err
	}

	return OuterHeader{
		Version:         OuterVersionV1,
		DstPeerID:       normalized.DstPeerID,
		SrcPeerID:       normalized.SenderPeerID,
		MsgID:           normalized.MsgID,
		ExpiresAtUnixMs: normalized.ExpiresAtUnixMs,
		Scheme:          SchemePeerE2EV1,
	}, nil
}

// BuildOuterAAD encodes the trusted outer fields bound into peer_e2e_v1 AAD.
func BuildOuterAAD(o OuterHeader) ([]byte, error) {
	normalized, err := normalizeOuter(o)
	if err != nil {
		return nil, err
	}

	out := make([]byte, 0, 96)
	out = AppendU64Field(out, outerTagVersion, normalized.Version)
	out, err = AppendASCIIField(out, outerTagDstPeerID, normalized.DstPeerID)
	if err != nil {
		return nil, err
	}
	out, err = AppendASCIIField(out, outerTagMsgID, normalized.MsgID)
	if err != nil {
		return nil, err
	}
	out = AppendU64Field(out, outerTagExpiresAt, normalized.ExpiresAtUnixMs)
	out, err = AppendASCIIField(out, outerTagScheme, normalized.Scheme)
	if err != nil {
		return nil, err
	}
	return out, nil
}

func normalizeOuter(o OuterHeader) (OuterHeader, error) {
	if o.Version != OuterVersionV1 {
		return OuterHeader{}, fmt.Errorf("%w: %d", ErrUnsupportedVersion, o.Version)
	}

	dstPeerID, err := CanonicalizePeerID(o.DstPeerID)
	if err != nil {
		return OuterHeader{}, err
	}

	srcPeerID := ""
	if o.SrcPeerID != "" {
		srcPeerID, err = CanonicalizePeerID(o.SrcPeerID)
		if err != nil {
			return OuterHeader{}, err
		}
	}

	msgID, err := CanonicalizeMsgID(o.MsgID)
	if err != nil {
		return OuterHeader{}, err
	}

	if o.ExpiresAtUnixMs == 0 {
		return OuterHeader{}, fmt.Errorf("%w: missing expires_at", ErrInvalidFieldValue)
	}
	if o.Scheme != SchemePeerE2EV1 {
		return OuterHeader{}, fmt.Errorf("%w: %q", ErrInvalidScheme, o.Scheme)
	}

	return OuterHeader{
		Version:         o.Version,
		DstPeerID:       dstPeerID,
		SrcPeerID:       srcPeerID,
		MsgID:           msgID,
		ExpiresAtUnixMs: o.ExpiresAtUnixMs,
		Scheme:          o.Scheme,
		Ciphertext:      append([]byte(nil), o.Ciphertext...),
	}, nil
}
