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

import (
	"bytes"
	"crypto/ed25519"
	"crypto/sha256"
	"errors"
	"fmt"
	"os"

	"github.com/miopunch/miopunch/internal/pocv1/peere2e"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

// AuthorityHandleJoinRequest seals and caches one enroll response for a join request.
func AuthorityHandleJoinRequest(
	handled *persist.Store,
	networkID string,
	requestMsgID string,
	opened wire.OpenedMessage,
	authorityPriv ed25519.PrivateKey,
	authorityX25519Pub []byte,
	response EnrollResponse,
) ([]byte, bool, error) {
	if handled == nil {
		return nil, false, fmt.Errorf("%w: nil persistence store", ErrInvalidJoinRequest)
	}
	normalizedReq, err := UnmarshalJoinRequest(opened.Inner.Body)
	if err != nil {
		return nil, false, wrapError(StageJoinRequest, Evidence{
			NetworkID: networkID,
			MsgID:     requestMsgID,
			JoinTopic: opened.Outer.MsgID,
		}, err)
	}
	if err := VerifyJoinRequest(normalizedReq); err != nil {
		return nil, false, wrapError(StageJoinRequest, Evidence{
			NetworkID: networkID,
			MsgID:     requestMsgID,
			JoinTopic: opened.Outer.MsgID,
		}, err)
	}
	if err := verifyJoinRequestAdmission(opened, normalizedReq); err != nil {
		return nil, false, wrapError(StageJoinRequest, Evidence{
			NetworkID:  networkID,
			InviteID:   normalizedReq.InviteID,
			MsgID:      requestMsgID,
			ReplyTopic: normalizedReq.ReplyTopic,
		}, err)
	}

	fp := fingerprintJoinRequest(normalizedReq)
	responseBody, err := response.MarshalBinary()
	if err != nil {
		return nil, false, wrapError(StageAuthority, Evidence{
			NetworkID: networkID,
			MsgID:     requestMsgID,
			InviteID:  normalizedReq.InviteID,
		}, err)
	}

	cached, err := handled.LoadEnrollHandledRequest(networkID, requestMsgID)
	switch {
	case err == nil:
		if !sameFingerprint(cached.RequestFingerprint, fingerprintBytes(fp)) {
			return nil, false, wrapError(StageAuthority, Evidence{
				NetworkID: networkID,
				MsgID:     requestMsgID,
			}, ErrRequestFingerprintMismatch)
		}
		return append([]byte(nil), cached.ResponseCiphertext...), true, nil
	case !errors.Is(err, os.ErrNotExist):
		return nil, false, wrapError(StageAuthority, Evidence{
			NetworkID: networkID,
			MsgID:     requestMsgID,
			InviteID:  normalizedReq.InviteID,
		}, err)
	}

	sealed, err := sealEnrollResponse(opened, normalizedReq, authorityPriv, authorityX25519Pub, responseBody)
	if err != nil {
		return nil, false, err
	}

	record := persist.EnrollHandledRequest{
		MsgID:              requestMsgID,
		RequestFingerprint: fingerprintBytes(fp),
		ResponseCiphertext: sealed,
	}
	if err := handled.StoreEnrollHandledRequest(networkID, record); err != nil {
		return nil, false, wrapError(StageAuthority, Evidence{
			NetworkID: networkID,
			MsgID:     requestMsgID,
			InviteID:  normalizedReq.InviteID,
		}, err)
	}
	return sealed, false, nil
}

// JoinerPersistBootstrap converts an enroll response into persistence handoff.
func JoinerPersistBootstrap(store *persist.Store, response EnrollResponse) error {
	if store == nil {
		return wrapError(StagePersistence, Evidence{}, fmt.Errorf("nil persistence store"))
	}
	joined, err := response.JoinedBootstrap()
	if err != nil {
		return wrapError(StagePersistence, Evidence{
			NetworkID: response.SelfMemberCredential.NetworkID,
		}, err)
	}
	if err := store.PersistJoinedBootstrap(joined); err != nil {
		return wrapError(StagePersistence, Evidence{
			NetworkID: joined.NetworkID,
		}, err)
	}
	return nil
}

func sealEnrollResponse(
	opened wire.OpenedMessage,
	req JoinRequest,
	authorityPriv ed25519.PrivateKey,
	authorityX25519Pub []byte,
	responseBody []byte,
) ([]byte, error) {
	if len(authorityX25519Pub) != 32 {
		return nil, fmt.Errorf("%w: invalid authority x25519 public key length: %d", ErrInvalidInviteCode, len(authorityX25519Pub))
	}
	authorityPeerID, err := wire.PeerIDFromEd25519Pub(authorityPriv.Public().(ed25519.PublicKey))
	if err != nil {
		return nil, err
	}
	recipientPeerID, err := req.PeerID()
	if err != nil {
		return nil, err
	}

	msgID := opened.Outer.MsgID
	outer := wire.OuterHeader{
		Version:         wire.OuterVersionV1,
		DstPeerID:       recipientPeerID,
		SrcPeerID:       authorityPeerID,
		MsgID:           msgID,
		ExpiresAtUnixMs: opened.Outer.ExpiresAtUnixMs,
		Scheme:          wire.SchemePeerE2EV1,
	}
	inner := wire.InnerMessage{
		DstPeerID:       recipientPeerID,
		MsgID:           msgID,
		CreatedAtUnixMs: opened.Inner.CreatedAtUnixMs,
		ExpiresAtUnixMs: opened.Inner.ExpiresAtUnixMs,
		SenderPeerID:    authorityPeerID,
		SenderEd25519:   append([]byte(nil), authorityPriv.Public().(ed25519.PublicKey)...),
		Kind:            wire.KindEnrollResponse,
		InReplyTo:       opened.Inner.MsgID,
		Body:            append([]byte(nil), responseBody...),
	}
	if err := wire.SignInner(authorityPriv, &inner); err != nil {
		return nil, err
	}
	sealed, err := peere2e.Seal(outer, inner, req.RequesterX25519Pub, peere2e.SealOptions{})
	if err != nil {
		return nil, err
	}
	return sealed.MarshalBinary()
}

func fingerprintJoinRequest(req JoinRequest) RequestFingerprint {
	sum := sha256.Sum256(mustMarshalJoinRequestBody(req))
	return RequestFingerprint{
		InviteID:            req.InviteID,
		RequesterEd25519Pub: append([]byte(nil), req.RequesterEd25519Pub...),
		RequesterX25519Pub:  append([]byte(nil), req.RequesterX25519Pub...),
		ReplyTopic:          req.ReplyTopic,
		DeviceName:          req.DeviceName,
		Platform:            req.Platform,
		CreatedAtUnixMs:     req.CreatedAtUnixMs,
		ExpiresAtUnixMs:     req.ExpiresAtUnixMs,
		BodySHA256:          sum,
	}
}

func fingerprintBytes(fp RequestFingerprint) []byte {
	out := make([]byte, 0, 160)
	out = append(out, fp.InviteID...)
	out = append(out, 0x00)
	out = append(out, fp.RequesterEd25519Pub...)
	out = append(out, fp.RequesterX25519Pub...)
	out = append(out, 0x00)
	out = append(out, fp.ReplyTopic...)
	out = append(out, 0x00)
	out = append(out, fp.DeviceName...)
	out = append(out, 0x00)
	out = append(out, fp.Platform...)
	out = append(out, 0x00)
	out = append(out, fp.BodySHA256[:]...)
	return out
}

func sameFingerprint(want []byte, got []byte) bool {
	return string(want) == string(got)
}

func mustMarshalJoinRequestBody(req JoinRequest) []byte {
	body, _ := req.MarshalBinary()
	return body
}

func verifyJoinRequestAdmission(opened wire.OpenedMessage, req JoinRequest) error {
	if opened.Inner.Kind != wire.KindJoinRequest {
		return fmt.Errorf("%w: unexpected kind %q", ErrInvalidJoinRequest, opened.Inner.Kind)
	}
	if !bytes.Equal(opened.Inner.SenderEd25519, req.RequesterEd25519Pub) {
		return fmt.Errorf(
			"%w: sender_ed25519 does not match requester_ed25519_pub",
			ErrJoinRequestSenderMismatch,
		)
	}

	requesterPeerID, err := req.PeerID()
	if err != nil {
		return err
	}
	if opened.Inner.SenderPeerID != requesterPeerID {
		return fmt.Errorf(
			"%w: sender_peer_id does not match requester_ed25519_pub",
			ErrJoinRequestSenderMismatch,
		)
	}
	return nil
}
