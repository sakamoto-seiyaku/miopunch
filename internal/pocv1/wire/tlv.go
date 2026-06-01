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
	"encoding/binary"
	"fmt"
	"maps"
)

// DecodedField is one strict-decoded TLV field.
type DecodedField struct {
	Tag   uint64
	Value []byte
}

// AppendBytesField appends one TLV bytes field to dst.
func AppendBytesField(dst []byte, tag uint64, value []byte) []byte {
	dst = appendUvarint(dst, tag)
	dst = appendUvarint(dst, uint64(len(value)))
	return append(dst, value...)
}

// AppendU64Field appends one TLV u64 field to dst using canonical uvarint
// value encoding.
func AppendU64Field(dst []byte, tag uint64, value uint64) []byte {
	var buf [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(buf[:], value)
	return AppendBytesField(dst, tag, buf[:n])
}

// AppendASCIIField appends one TLV ASCII field to dst.
func AppendASCIIField(dst []byte, tag uint64, value string) ([]byte, error) {
	if err := validateASCII([]byte(value)); err != nil {
		return nil, err
	}
	return AppendBytesField(dst, tag, []byte(value)), nil
}

// DecodeFieldsStrict decodes TLV fields and rejects unknown tags, duplicates,
// non-canonical uvarints, out-of-order tags, and truncation.
func DecodeFieldsStrict(data []byte, allowedTags ...uint64) ([]DecodedField, error) {
	allowed := make(map[uint64]struct{}, len(allowedTags))
	for _, tag := range allowedTags {
		allowed[tag] = struct{}{}
	}
	return decodeFieldsStrict(data, allowed)
}

// DecodeU64Field decodes one TLV u64 field value using canonical uvarint rules.
func DecodeU64Field(field DecodedField) (uint64, error) {
	value, n, err := decodeUvarintCanonical(field.Value)
	if err != nil {
		return 0, err
	}
	if n != len(field.Value) {
		return 0, fmt.Errorf("%w: tag=%d trailing bytes", ErrInvalidFieldValue, field.Tag)
	}
	return value, nil
}

// DecodeASCIIField decodes one TLV ASCII field value.
func DecodeASCIIField(field DecodedField) (string, error) {
	if err := validateASCII(field.Value); err != nil {
		return "", fmt.Errorf("tag=%d: %w", field.Tag, err)
	}
	return string(field.Value), nil
}

func decodeFieldsStrict(data []byte, allowed map[uint64]struct{}) ([]DecodedField, error) {
	if len(data) == 0 {
		return nil, nil
	}

	out := make([]DecodedField, 0, len(allowed))
	seen := make(map[uint64]struct{}, len(allowed))
	var lastTag uint64
	offset := 0
	for offset < len(data) {
		tag, n, err := decodeUvarintCanonical(data[offset:])
		if err != nil {
			return nil, err
		}
		offset += n

		length, n, err := decodeUvarintCanonical(data[offset:])
		if err != nil {
			return nil, err
		}
		offset += n

		if _, ok := allowed[tag]; !ok {
			return nil, fmt.Errorf("%w: tag=%d", ErrUnknownTag, tag)
		}
		if _, ok := seen[tag]; ok {
			return nil, fmt.Errorf("%w: tag=%d", ErrDuplicateTag, tag)
		}
		if len(seen) > 0 && tag <= lastTag {
			return nil, fmt.Errorf("%w: tag=%d after tag=%d", ErrOutOfOrderField, tag, lastTag)
		}
		lastTag = tag
		seen[tag] = struct{}{}

		if length > uint64(len(data)-offset) {
			return nil, fmt.Errorf("%w: tag=%d len=%d remaining=%d", ErrTruncatedTLV, tag, length, len(data)-offset)
		}

		value := append([]byte(nil), data[offset:offset+int(length)]...)
		offset += int(length)
		out = append(out, DecodedField{
			Tag:   tag,
			Value: value,
		})
	}

	return out, nil
}

func indexFields(fields []DecodedField) map[uint64]DecodedField {
	out := make(map[uint64]DecodedField, len(fields))
	for _, field := range fields {
		out[field.Tag] = field
	}
	return out
}

func cloneAllowed(allowed map[uint64]struct{}) map[uint64]struct{} {
	return maps.Clone(allowed)
}

func appendUvarint(dst []byte, value uint64) []byte {
	var buf [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(buf[:], value)
	return append(dst, buf[:n]...)
}

func decodeUvarintCanonical(data []byte) (uint64, int, error) {
	value, n := binary.Uvarint(data)
	switch {
	case n == 0:
		return 0, 0, fmt.Errorf("%w: truncated uvarint", ErrTruncatedTLV)
	case n < 0:
		return 0, 0, fmt.Errorf("%w: overflow", ErrNonCanonicalUvarint)
	}

	var buf [binary.MaxVarintLen64]byte
	m := binary.PutUvarint(buf[:], value)
	if m != n || !bytes.Equal(buf[:m], data[:n]) {
		return 0, 0, fmt.Errorf("%w", ErrNonCanonicalUvarint)
	}
	return value, n, nil
}

func validateASCII(data []byte) error {
	for _, b := range data {
		if b > 0x7f {
			return fmt.Errorf("%w: 0x%02x", ErrInvalidASCII, b)
		}
	}
	return nil
}
