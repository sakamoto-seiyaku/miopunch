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

import "errors"

var (
	ErrInvalidConfig            = errors.New("invalid punch config")
	ErrInvalidOffer             = errors.New("invalid dial offer")
	ErrInvalidAnswer            = errors.New("invalid dial answer")
	ErrRemoteSenderMismatch     = errors.New("remote sender mismatch")
	ErrRemoteCredentialMismatch = errors.New("remote credential mismatch")
	ErrRemoteRosterMismatch     = errors.New("remote roster mismatch")
	ErrRemoteAuthorityVerify    = errors.New("remote authority verification failed")
	ErrTargetOffline            = errors.New("dial target is not online")
	ErrTargetNotInRoster        = errors.New("dial target not found in trusted roster")
	ErrNoCandidatePairs         = errors.New("no candidate pairs available")
	ErrAttemptBudgetExceeded    = errors.New("attempt budget exceeded")
	ErrUnsupportedP2PNetwork    = errors.New("unsupported p2p network")
)
