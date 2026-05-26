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

package punch

import (
	"bytes"
	"encoding/binary"
	"fmt"

	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
)

const (
	dialTagDialID           = 1
	dialTagPunchToken       = 2
	dialTagCandidates       = 3
	dialTagMemberCredential = 4

	candidateTagKind = 1
	candidateTagAddr = 2
)

var dialAllowedTags = []uint64{
	dialTagDialID,
	dialTagPunchToken,
	dialTagCandidates,
	dialTagMemberCredential,
}

var candidateAllowedTags = []uint64{
	candidateTagKind,
	candidateTagAddr,
}

func (o DialOffer) MarshalBinary() ([]byte, error) {
	normalized, err := normalizeDialOffer(o)
	if err != nil {
		return nil, err
	}
	return marshalDialMessage(
		normalized.DialID,
		normalized.PunchToken,
		normalized.Candidates,
		normalized.MemberCredential,
	)
}

func UnmarshalDialOffer(data []byte) (DialOffer, error) {
	dialID, punchToken, candidates, memberCredential, err := unmarshalDialMessage(data)
	if err != nil {
		return DialOffer{}, err
	}
	return normalizeDialOffer(DialOffer{
		DialID:           dialID,
		PunchToken:       punchToken,
		Candidates:       candidates,
		MemberCredential: memberCredential,
	})
}

func (a DialAnswer) MarshalBinary() ([]byte, error) {
	normalized, err := normalizeDialAnswer(a)
	if err != nil {
		return nil, err
	}
	return marshalDialMessage(
		normalized.DialID,
		normalized.PunchToken,
		normalized.Candidates,
		normalized.MemberCredential,
	)
}

func UnmarshalDialAnswer(data []byte) (DialAnswer, error) {
	dialID, punchToken, candidates, memberCredential, err := unmarshalDialMessage(data)
	if err != nil {
		return DialAnswer{}, err
	}
	return normalizeDialAnswer(DialAnswer{
		DialID:           dialID,
		PunchToken:       punchToken,
		Candidates:       candidates,
		MemberCredential: memberCredential,
	})
}

func marshalDialMessage(dialID string, punchToken []byte, candidates []Candidate, memberCredential []byte) ([]byte, error) {
	out := make([]byte, 0, 256)
	var err error
	out, err = pocwire.AppendASCIIField(out, dialTagDialID, dialID)
	if err != nil {
		return nil, err
	}
	out = pocwire.AppendBytesField(out, dialTagPunchToken, punchToken)
	out = pocwire.AppendBytesField(out, dialTagCandidates, marshalCandidates(candidates))
	out = pocwire.AppendBytesField(out, dialTagMemberCredential, memberCredential)
	return out, nil
}

func unmarshalDialMessage(data []byte) (string, []byte, []Candidate, []byte, error) {
	fields, err := pocwire.DecodeFieldsStrict(data, dialAllowedTags...)
	if err != nil {
		return "", nil, nil, nil, err
	}
	index := indexPocFields(fields)
	dialID, err := requireASCII(index, dialTagDialID, "dial_id")
	if err != nil {
		return "", nil, nil, nil, err
	}
	punchToken, err := requireBytes(index, dialTagPunchToken, "punch_token")
	if err != nil {
		return "", nil, nil, nil, err
	}
	candidateBytes, err := requireBytes(index, dialTagCandidates, "candidates")
	if err != nil {
		return "", nil, nil, nil, err
	}
	candidates, err := unmarshalCandidates(candidateBytes)
	if err != nil {
		return "", nil, nil, nil, err
	}
	memberCredential, err := requireBytes(index, dialTagMemberCredential, "member_credential")
	if err != nil {
		return "", nil, nil, nil, err
	}
	return dialID, punchToken, candidates, memberCredential, nil
}

func marshalCandidates(candidates []Candidate) []byte {
	out := make([]byte, 0, len(candidates)*32)
	for _, candidate := range candidates {
		entry := make([]byte, 0, 64)
		entry, _ = pocwire.AppendASCIIField(entry, candidateTagKind, string(candidate.Kind))
		entry, _ = pocwire.AppendASCIIField(entry, candidateTagAddr, candidate.Addr)
		out = appendLengthPrefixed(out, entry)
	}
	return out
}

func unmarshalCandidates(data []byte) ([]Candidate, error) {
	entries, err := decodeLengthPrefixedList(data)
	if err != nil {
		return nil, err
	}
	candidates := make([]Candidate, 0, len(entries))
	for i, entry := range entries {
		fields, err := pocwire.DecodeFieldsStrict(entry, candidateAllowedTags...)
		if err != nil {
			return nil, fmt.Errorf("candidate %d: %w", i, err)
		}
		index := indexPocFields(fields)
		kind, err := requireASCII(index, candidateTagKind, "kind")
		if err != nil {
			return nil, fmt.Errorf("candidate %d: %w", i, err)
		}
		addr, err := requireASCII(index, candidateTagAddr, "addr")
		if err != nil {
			return nil, fmt.Errorf("candidate %d: %w", i, err)
		}
		candidates = append(candidates, Candidate{
			Kind: CandidateKind(kind),
			Addr: addr,
		})
	}
	return normalizeCandidates(candidates)
}

func normalizeDialOffer(in DialOffer) (DialOffer, error) {
	dialID, err := pocwire.CanonicalizeMsgID(in.DialID)
	if err != nil {
		return DialOffer{}, fmt.Errorf("%w: canonicalize dial_id: %w", ErrInvalidOffer, err)
	}
	if len(in.PunchToken) != 16 {
		return DialOffer{}, fmt.Errorf("%w: invalid punch token length: %d", ErrInvalidOffer, len(in.PunchToken))
	}
	candidates, err := normalizeCandidates(in.Candidates)
	if err != nil {
		return DialOffer{}, fmt.Errorf("%w: %w", ErrInvalidOffer, err)
	}
	if len(in.MemberCredential) == 0 {
		return DialOffer{}, fmt.Errorf("%w: missing member credential", ErrInvalidOffer)
	}
	return DialOffer{
		DialID:           dialID,
		PunchToken:       append([]byte(nil), in.PunchToken...),
		Candidates:       candidates,
		MemberCredential: append([]byte(nil), in.MemberCredential...),
	}, nil
}

func normalizeDialAnswer(in DialAnswer) (DialAnswer, error) {
	dialID, err := pocwire.CanonicalizeMsgID(in.DialID)
	if err != nil {
		return DialAnswer{}, fmt.Errorf("%w: canonicalize dial_id: %w", ErrInvalidAnswer, err)
	}
	if len(in.PunchToken) != 16 {
		return DialAnswer{}, fmt.Errorf("%w: invalid punch token length: %d", ErrInvalidAnswer, len(in.PunchToken))
	}
	candidates, err := normalizeCandidates(in.Candidates)
	if err != nil {
		return DialAnswer{}, fmt.Errorf("%w: %w", ErrInvalidAnswer, err)
	}
	if len(in.MemberCredential) == 0 {
		return DialAnswer{}, fmt.Errorf("%w: missing member credential", ErrInvalidAnswer)
	}
	return DialAnswer{
		DialID:           dialID,
		PunchToken:       append([]byte(nil), in.PunchToken...),
		Candidates:       candidates,
		MemberCredential: append([]byte(nil), in.MemberCredential...),
	}, nil
}

func indexPocFields(fields []pocwire.DecodedField) map[uint64]pocwire.DecodedField {
	out := make(map[uint64]pocwire.DecodedField, len(fields))
	for _, field := range fields {
		out[field.Tag] = field
	}
	return out
}

func requireASCII(index map[uint64]pocwire.DecodedField, tag uint64, name string) (string, error) {
	field, ok := index[tag]
	if !ok {
		return "", fmt.Errorf("%w: missing %s", pocwire.ErrInvalidFieldValue, name)
	}
	return pocwire.DecodeASCIIField(field)
}

func requireBytes(index map[uint64]pocwire.DecodedField, tag uint64, name string) ([]byte, error) {
	field, ok := index[tag]
	if !ok {
		return nil, fmt.Errorf("%w: missing %s", pocwire.ErrInvalidFieldValue, name)
	}
	return append([]byte(nil), field.Value...), nil
}

func appendLengthPrefixed(dst []byte, value []byte) []byte {
	var buf [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(buf[:], uint64(len(value)))
	dst = append(dst, buf[:n]...)
	return append(dst, value...)
}

func decodeLengthPrefixedList(data []byte) ([][]byte, error) {
	out := make([][]byte, 0, 4)
	offset := 0
	for offset < len(data) {
		length, n, err := decodeCanonicalUvarint(data[offset:])
		if err != nil {
			return nil, err
		}
		offset += n
		if length > uint64(len(data)-offset) {
			return nil, fmt.Errorf("%w: candidates entry length %d exceeds remaining %d", pocwire.ErrTruncatedTLV, length, len(data)-offset)
		}
		out = append(out, append([]byte(nil), data[offset:offset+int(length)]...))
		offset += int(length)
	}
	return out, nil
}

func decodeCanonicalUvarint(data []byte) (uint64, int, error) {
	value, n := binary.Uvarint(data)
	switch {
	case n == 0:
		return 0, 0, fmt.Errorf("%w: truncated uvarint", pocwire.ErrTruncatedTLV)
	case n < 0:
		return 0, 0, fmt.Errorf("%w: overflow", pocwire.ErrNonCanonicalUvarint)
	}
	var buf [binary.MaxVarintLen64]byte
	m := binary.PutUvarint(buf[:], value)
	if m != n || !bytes.Equal(buf[:m], data[:n]) {
		return 0, 0, fmt.Errorf("%w", pocwire.ErrNonCanonicalUvarint)
	}
	return value, n, nil
}
