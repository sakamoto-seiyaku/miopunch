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

// AdmissionOptions carries local drop-only admission hooks.
type AdmissionOptions struct {
	NowUnixMs uint64
	SeenMsgID func(msgID string) bool
}

// OpenedMessage is one decrypted outer/inner pair ready for admission.
type OpenedMessage struct {
	Outer OuterHeader
	Inner InnerMessage
}

// Admit verifies the current v1 outer/inner invariants and local hooks.
func Admit(msg OpenedMessage, opts AdmissionOptions) error {
	outer, err := normalizeOuter(msg.Outer)
	if err != nil {
		return fmt.Errorf("normalize outer header: %w", err)
	}
	inner, err := normalizeInner(msg.Inner, true)
	if err != nil {
		return fmt.Errorf("normalize inner message: %w", err)
	}
	if err := VerifyInner(inner); err != nil {
		return fmt.Errorf("verify inner message: %w", err)
	}

	if outer.DstPeerID != inner.DstPeerID {
		return fmt.Errorf("%w: dst mismatch", ErrOuterInnerMismatch)
	}
	if outer.MsgID != inner.MsgID {
		return fmt.Errorf("%w: msg_id mismatch", ErrOuterInnerMismatch)
	}
	if outer.ExpiresAtUnixMs != inner.ExpiresAtUnixMs {
		return fmt.Errorf("%w: expires_at mismatch", ErrOuterInnerMismatch)
	}

	if opts.NowUnixMs != 0 && opts.NowUnixMs > inner.ExpiresAtUnixMs {
		return fmt.Errorf("%w: now=%d expires_at=%d", ErrExpired, opts.NowUnixMs, inner.ExpiresAtUnixMs)
	}
	if opts.SeenMsgID != nil && opts.SeenMsgID(inner.MsgID) {
		return fmt.Errorf("%w: msg_id=%s", ErrReplay, inner.MsgID)
	}

	return nil
}
