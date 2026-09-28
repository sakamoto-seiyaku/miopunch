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

package enroll

import "errors"

var (
	// ErrInvalidInviteCode reports a malformed MPINV1 invite code.
	ErrInvalidInviteCode = errors.New("invalid invite code")
	// ErrInvalidBrokerEndpoint reports a malformed runtime broker endpoint.
	ErrInvalidBrokerEndpoint = errors.New("invalid broker endpoint")
	// ErrInvalidJoinRequest reports a malformed join request.
	ErrInvalidJoinRequest = errors.New("invalid join request")
	// ErrJoinRequestSenderMismatch reports an admitted sender/body identity mismatch.
	ErrJoinRequestSenderMismatch = errors.New("join request sender mismatch")
	// ErrInvalidMemberCredential reports a malformed member credential.
	ErrInvalidMemberCredential = errors.New("invalid member credential")
	// ErrInvalidRosterSnapshot reports a malformed roster snapshot.
	ErrInvalidRosterSnapshot = errors.New("invalid roster snapshot")
	// ErrRequestFingerprintMismatch reports a replayed msg_id with mismatched content.
	ErrRequestFingerprintMismatch = errors.New("request fingerprint mismatch")
)

// Stage identifies the local enrollment stage attached to a typed error.
type Stage string

const (
	// StageInvite covers invite-code decode and verification.
	StageInvite Stage = "invite"
	// StageJoinRequest covers join-request decode and verification.
	StageJoinRequest Stage = "join_request"
	// StageMemberCredential covers member-credential build and verification.
	StageMemberCredential Stage = "member_credential"
	// StageAuthority covers approve/enroll handling and replay caching.
	StageAuthority Stage = "authority"
	// StagePersistence covers joiner bootstrap handoff into persistence.
	StagePersistence Stage = "persistence"
)

// Evidence carries structured enrollment debugging context.
type Evidence struct {
	NetworkID          string
	InviteID           string
	MsgID              string
	PeerID             string
	ReplyTopic         string
	JoinTopic          string
	RequestFingerprint string
}

// Error wraps a stage-local enrollment failure with structured evidence.
type Error struct {
	Stage    Stage
	Evidence Evidence
	Err      error
}

// Error reports the wrapped enrollment failure.
func (e *Error) Error() string {
	if e == nil {
		return "<nil>"
	}
	if e.Err == nil {
		return string(e.Stage)
	}
	return string(e.Stage) + ": " + e.Err.Error()
}

// Unwrap returns the underlying cause.
func (e *Error) Unwrap() error {
	if e == nil {
		return nil
	}
	return e.Err
}

func wrapError(stage Stage, evidence Evidence, err error) error {
	if err == nil {
		return nil
	}
	return &Error{
		Stage:    stage,
		Evidence: evidence,
		Err:      err,
	}
}
