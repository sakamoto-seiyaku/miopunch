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

import "errors"

var (
	// ErrMalformedTLV reports structurally invalid TLV input.
	ErrMalformedTLV = errors.New("malformed tlv")
	// ErrUnknownTag reports a TLV tag outside the schema allowlist.
	ErrUnknownTag = errors.New("unknown tag")
	// ErrDuplicateTag reports a duplicate TLV tag.
	ErrDuplicateTag = errors.New("duplicate tag")
	// ErrNonCanonicalUvarint reports a non-minimal uvarint encoding.
	ErrNonCanonicalUvarint = errors.New("non-canonical uvarint")
	// ErrOutOfOrderField reports tags that are not strictly increasing.
	ErrOutOfOrderField = errors.New("out-of-order field")
	// ErrTruncatedTLV reports a truncated TLV field or payload.
	ErrTruncatedTLV = errors.New("truncated tlv")
	// ErrInvalidASCII reports non-ASCII data in an ASCII field.
	ErrInvalidASCII = errors.New("invalid ascii")
	// ErrInvalidFieldValue reports a syntactically present but invalid field.
	ErrInvalidFieldValue = errors.New("invalid field value")
	// ErrUnsupportedVersion reports an unsupported outer header version.
	ErrUnsupportedVersion = errors.New("unsupported version")
	// ErrInvalidScheme reports an invalid outer encryption scheme.
	ErrInvalidScheme = errors.New("invalid scheme")
	// ErrUnsupportedKind reports an inner kind outside the allowlist.
	ErrUnsupportedKind = errors.New("unsupported kind")
	// ErrInvalidSignature reports a failed Ed25519 signature check.
	ErrInvalidSignature = errors.New("invalid signature")
	// ErrOuterInnerMismatch reports mismatched outer and inner envelope fields.
	ErrOuterInnerMismatch = errors.New("outer/inner mismatch")
	// ErrExpired reports an expired current v1 message.
	ErrExpired = errors.New("message expired")
	// ErrReplay reports a replayed message admitted by the local hook.
	ErrReplay = errors.New("replayed message")
)
