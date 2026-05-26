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
	"context"
	"errors"
	"fmt"

	"github.com/miopunch/miopunch/internal/pocv1/enroll"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
	signalmqtt "github.com/miopunch/miopunch/internal/signaling/mqtt"
)

func resolveTarget(cfg LoadedConfig, target Target) (trustedRemote, error) {
	peerID, err := pocwire.CanonicalizePeerID(target.PeerID)
	if err != nil {
		return trustedRemote{}, fmt.Errorf("%w: canonicalize target peer_id: %w", ErrInvalidConfig, err)
	}
	if !targetOnline(cfg.Discover, peerID) {
		return trustedRemote{}, ErrTargetOffline
	}
	entry, ok := cfg.TrustedRosterByID[peerID]
	if !ok {
		return trustedRemote{}, ErrTargetNotInRoster
	}
	return trustedRemoteFromRoster(cfg, entry)
}

func trustedRemoteFromRoster(cfg LoadedConfig, entry persist.RosterEntry) (trustedRemote, error) {
	credential, err := enroll.UnmarshalMemberCredential(entry.MemberCredential)
	if err != nil {
		return trustedRemote{}, fmt.Errorf("unmarshal roster member credential: %w", err)
	}
	if err := enroll.VerifyMemberCredential(credential, cfg.AuthorityEd25519Pub); err != nil {
		return trustedRemote{}, fmt.Errorf("%w: %w", ErrRemoteAuthorityVerify, err)
	}
	peerID, err := credential.PeerID()
	if err != nil {
		return trustedRemote{}, fmt.Errorf("derive roster peer_id: %w", err)
	}
	if peerID != entry.PeerID {
		return trustedRemote{}, fmt.Errorf("%w: roster peer_id mismatch", ErrRemoteRosterMismatch)
	}
	return trustedRemote{
		PeerID:           peerID,
		MemberCredential: append([]byte(nil), entry.MemberCredential...),
		Credential:       credential,
		X25519PublicKey:  append([]byte(nil), credential.SubjectX25519Pub...),
	}, nil
}

func verifyOffer(cfg LoadedConfig, opened signalmqtt.OpenedPeerMessage) (DialOffer, trustedRemote, error) {
	if opened.Inner.Kind != pocwire.KindDialOffer {
		return DialOffer{}, trustedRemote{}, fmt.Errorf("%w: unexpected inner kind %q", ErrInvalidOffer, opened.Inner.Kind)
	}
	offer, err := UnmarshalDialOffer(opened.Inner.Body)
	if err != nil {
		return DialOffer{}, trustedRemote{}, err
	}
	remote, err := verifyRemoteAssertion(cfg, opened.Inner, offer.MemberCredential)
	if err != nil {
		return DialOffer{}, trustedRemote{}, err
	}
	return offer, remote, nil
}

func verifyAnswer(
	cfg LoadedConfig,
	opened signalmqtt.OpenedPeerMessage,
	wantDialID string,
	wantPunchToken []byte,
	wantInReplyTo string,
) (DialAnswer, trustedRemote, error) {
	if opened.Inner.Kind != pocwire.KindDialAnswer {
		return DialAnswer{}, trustedRemote{}, fmt.Errorf("%w: unexpected inner kind %q", ErrInvalidAnswer, opened.Inner.Kind)
	}
	answer, err := UnmarshalDialAnswer(opened.Inner.Body)
	if err != nil {
		return DialAnswer{}, trustedRemote{}, err
	}
	if answer.DialID != wantDialID {
		return DialAnswer{}, trustedRemote{}, fmt.Errorf("%w: dial_id mismatch", ErrInvalidAnswer)
	}
	if !bytes.Equal(answer.PunchToken, wantPunchToken) {
		return DialAnswer{}, trustedRemote{}, fmt.Errorf("%w: punch_token mismatch", ErrInvalidAnswer)
	}
	if opened.Inner.InReplyTo != wantInReplyTo {
		return DialAnswer{}, trustedRemote{}, fmt.Errorf("%w: in_reply_to mismatch", ErrInvalidAnswer)
	}
	remote, err := verifyRemoteAssertion(cfg, opened.Inner, answer.MemberCredential)
	if err != nil {
		return DialAnswer{}, trustedRemote{}, err
	}
	return answer, remote, nil
}

func verifyRemoteAssertion(cfg LoadedConfig, inner pocwire.InnerMessage, memberCredential []byte) (trustedRemote, error) {
	credential, err := enroll.UnmarshalMemberCredential(memberCredential)
	if err != nil {
		return trustedRemote{}, fmt.Errorf("%w: unmarshal member credential: %w", ErrRemoteCredentialMismatch, err)
	}
	derivedPeerID, err := credential.PeerID()
	if err != nil {
		return trustedRemote{}, fmt.Errorf("%w: derive peer_id: %w", ErrRemoteCredentialMismatch, err)
	}
	if inner.SenderPeerID != derivedPeerID {
		return trustedRemote{}, fmt.Errorf("%w: sender_peer_id=%s credential_peer_id=%s", ErrRemoteSenderMismatch, inner.SenderPeerID, derivedPeerID)
	}
	if !bytes.Equal(inner.SenderEd25519, credential.SubjectEd25519Pub) {
		return trustedRemote{}, fmt.Errorf("%w: sender_ed25519 mismatch", ErrRemoteSenderMismatch)
	}
	entry, ok := cfg.TrustedRosterByID[derivedPeerID]
	if !ok {
		return trustedRemote{}, ErrTargetNotInRoster
	}
	if !bytes.Equal(entry.MemberCredential, memberCredential) {
		return trustedRemote{}, fmt.Errorf("%w: roster credential bytes mismatch", ErrRemoteRosterMismatch)
	}
	if err := enroll.VerifyMemberCredential(credential, cfg.AuthorityEd25519Pub); err != nil {
		return trustedRemote{}, fmt.Errorf("%w: %w", ErrRemoteAuthorityVerify, err)
	}
	return trustedRemote{
		PeerID:           derivedPeerID,
		MemberCredential: append([]byte(nil), memberCredential...),
		Credential:       credential,
		X25519PublicKey:  append([]byte(nil), credential.SubjectX25519Pub...),
	}, nil
}

type trustedRemote struct {
	PeerID           string
	MemberCredential []byte
	Credential       enroll.MemberCredential
	X25519PublicKey  []byte
}

func isTimeoutError(err error) bool {
	return errors.Is(err, context.DeadlineExceeded) || errors.Is(err, ErrAttemptBudgetExceeded)
}
