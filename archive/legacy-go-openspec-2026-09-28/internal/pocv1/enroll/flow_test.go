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
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

func TestAuthorityHandleJoinRequestCachesResponseAcrossRestart(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	root := filepath.Join(t.TempDir(), "state")
	store, err := persist.Open(root)
	if err != nil {
		t.Fatalf("persist.Open() error = %v, want nil", err)
	}

	opened := mustOpenedJoinRequest(t, fx)
	first, hit, err := AuthorityHandleJoinRequest(store, fx.networkID, opened.Outer.MsgID, opened, fx.authorityPriv, fx.invite.AuthorityX25519Pub, fx.enrollResponse)
	if err != nil {
		t.Fatalf("AuthorityHandleJoinRequest(first) error = %v, want nil", err)
	}
	if hit {
		t.Fatalf("AuthorityHandleJoinRequest(first) hit = true, want false")
	}

	reopened, err := persist.Open(root)
	if err != nil {
		t.Fatalf("persist.Open(reopen) error = %v, want nil", err)
	}
	second, hit, err := AuthorityHandleJoinRequest(reopened, fx.networkID, opened.Outer.MsgID, opened, fx.authorityPriv, fx.invite.AuthorityX25519Pub, fx.enrollResponse)
	if err != nil {
		t.Fatalf("AuthorityHandleJoinRequest(second) error = %v, want nil", err)
	}
	if !hit {
		t.Fatalf("AuthorityHandleJoinRequest(second) hit = false, want true")
	}
	if !bytes.Equal(first, second) {
		t.Fatalf("AuthorityHandleJoinRequest() cached ciphertext mismatch")
	}
}

func TestAuthorityHandleJoinRequestRejectsSenderBodyMismatch(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	store, err := persist.Open(filepath.Join(t.TempDir(), "state"))
	if err != nil {
		t.Fatalf("persist.Open() error = %v, want nil", err)
	}

	opened := mustOpenedJoinRequest(t, fx)
	otherPriv := mustOtherJoinRequestSigner(t)
	otherPub := otherPriv.Public().(ed25519.PublicKey)
	otherPeerID, err := wire.PeerIDFromEd25519Pub(otherPub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(other) error = %v, want nil", err)
	}
	opened.Inner.SenderEd25519 = append([]byte(nil), otherPub...)
	opened.Inner.SenderPeerID = otherPeerID
	if err := wire.SignInner(otherPriv, &opened.Inner); err != nil {
		t.Fatalf("wire.SignInner(other) error = %v, want nil", err)
	}
	opened.Outer.SrcPeerID = otherPeerID

	_, _, err = AuthorityHandleJoinRequest(
		store,
		fx.networkID,
		opened.Outer.MsgID,
		opened,
		fx.authorityPriv,
		fx.invite.AuthorityX25519Pub,
		fx.enrollResponse,
	)
	if !errors.Is(err, ErrJoinRequestSenderMismatch) {
		t.Fatalf("AuthorityHandleJoinRequest(sender mismatch) error = %v, want %v", err, ErrJoinRequestSenderMismatch)
	}
}

func TestAuthorityHandleJoinRequestRejectsSenderPeerIDMismatch(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	store, err := persist.Open(filepath.Join(t.TempDir(), "state"))
	if err != nil {
		t.Fatalf("persist.Open() error = %v, want nil", err)
	}

	opened := mustOpenedJoinRequest(t, fx)
	otherPriv := mustOtherJoinRequestSigner(t)
	otherPub := otherPriv.Public().(ed25519.PublicKey)
	otherPeerID, err := wire.PeerIDFromEd25519Pub(otherPub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(other) error = %v, want nil", err)
	}
	opened.Inner.SenderPeerID = otherPeerID
	opened.Outer.SrcPeerID = otherPeerID

	_, _, err = AuthorityHandleJoinRequest(
		store,
		fx.networkID,
		opened.Outer.MsgID,
		opened,
		fx.authorityPriv,
		fx.invite.AuthorityX25519Pub,
		fx.enrollResponse,
	)
	if !errors.Is(err, ErrJoinRequestSenderMismatch) {
		t.Fatalf("AuthorityHandleJoinRequest(sender peer mismatch) error = %v, want %v", err, ErrJoinRequestSenderMismatch)
	}
}

func TestAuthorityHandleJoinRequestFailsClosedOnCorruptReplayCache(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	root := filepath.Join(t.TempDir(), "state")
	store, err := persist.Open(root)
	if err != nil {
		t.Fatalf("persist.Open() error = %v, want nil", err)
	}

	opened := mustOpenedJoinRequest(t, fx)
	if err := store.StoreEnrollHandledRequest(fx.networkID, persist.EnrollHandledRequest{
		MsgID:              opened.Outer.MsgID,
		RequestFingerprint: []byte("fingerprint"),
		ResponseCiphertext: []byte("ciphertext"),
	}); err != nil {
		t.Fatalf("StoreEnrollHandledRequest() error = %v, want nil", err)
	}

	cachePath := filepath.Join(root, "device", "enroll_handled", fx.networkID, opened.Outer.MsgID+".json")
	if err := os.WriteFile(cachePath, []byte("{"), 0o600); err != nil {
		t.Fatalf("WriteFile(%q) error = %v, want nil", cachePath, err)
	}

	_, _, err = AuthorityHandleJoinRequest(
		store,
		fx.networkID,
		opened.Outer.MsgID,
		opened,
		fx.authorityPriv,
		fx.invite.AuthorityX25519Pub,
		fx.enrollResponse,
	)
	if err == nil {
		t.Fatalf("AuthorityHandleJoinRequest(corrupt replay cache) error = nil, want non-nil")
	}
}

func TestAuthorityHandleJoinRequestRejectsFingerprintMismatch(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	root := filepath.Join(t.TempDir(), "state")
	store, err := persist.Open(root)
	if err != nil {
		t.Fatalf("persist.Open() error = %v, want nil", err)
	}

	opened := mustOpenedJoinRequest(t, fx)
	first, hit, err := AuthorityHandleJoinRequest(store, fx.networkID, opened.Outer.MsgID, opened, fx.authorityPriv, fx.invite.AuthorityX25519Pub, fx.enrollResponse)
	if err != nil {
		t.Fatalf("AuthorityHandleJoinRequest(first) error = %v, want nil", err)
	}
	if hit {
		t.Fatalf("AuthorityHandleJoinRequest(first) hit = true, want false")
	}
	if len(first) == 0 {
		t.Fatalf("AuthorityHandleJoinRequest(first) = empty response")
	}

	mutated := opened
	req := fx.joinRequest
	req.DeviceName = "beta"
	if err := SignJoinRequest(fx.requesterPriv, &req); err != nil {
		t.Fatalf("SignJoinRequest(mutated) error = %v, want nil", err)
	}
	body, err := req.MarshalBinary()
	if err != nil {
		t.Fatalf("JoinRequest.MarshalBinary(mutated) error = %v, want nil", err)
	}
	mutated.Inner.Body = body
	if err := wire.SignInner(fx.requesterPriv, &mutated.Inner); err != nil {
		t.Fatalf("wire.SignInner(mutated) error = %v, want nil", err)
	}

	_, _, err = AuthorityHandleJoinRequest(
		store,
		fx.networkID,
		mutated.Outer.MsgID,
		mutated,
		fx.authorityPriv,
		fx.invite.AuthorityX25519Pub,
		fx.enrollResponse,
	)
	if !errors.Is(err, ErrRequestFingerprintMismatch) {
		t.Fatalf("AuthorityHandleJoinRequest(fingerprint mismatch) error = %v, want %v", err, ErrRequestFingerprintMismatch)
	}
}

func TestAuthorityHandleJoinRequestRejectsUnexpectedKind(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	store, err := persist.Open(filepath.Join(t.TempDir(), "state"))
	if err != nil {
		t.Fatalf("persist.Open() error = %v, want nil", err)
	}

	opened := mustOpenedJoinRequest(t, fx)
	opened.Inner.Kind = wire.KindEnrollResponse
	if err := wire.SignInner(fx.requesterPriv, &opened.Inner); err != nil {
		t.Fatalf("wire.SignInner(unexpected kind) error = %v, want nil", err)
	}

	_, _, err = AuthorityHandleJoinRequest(
		store,
		fx.networkID,
		opened.Outer.MsgID,
		opened,
		fx.authorityPriv,
		fx.invite.AuthorityX25519Pub,
		fx.enrollResponse,
	)
	if !errors.Is(err, ErrInvalidJoinRequest) {
		t.Fatalf("AuthorityHandleJoinRequest(unexpected kind) error = %v, want %v", err, ErrInvalidJoinRequest)
	}
}

func TestAuthorityHandleJoinRequestRejectsInvalidPoP(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	store, err := persist.Open(filepath.Join(t.TempDir(), "state"))
	if err != nil {
		t.Fatalf("persist.Open() error = %v, want nil", err)
	}

	opened := mustOpenedJoinRequest(t, fx)
	req := fx.joinRequest
	req.ProofOfPossessionSig = append([]byte(nil), fx.joinRequest.ProofOfPossessionSig...)
	req.ProofOfPossessionSig[0] ^= 0xff
	body, err := req.MarshalBinary()
	if err != nil {
		t.Fatalf("JoinRequest.MarshalBinary(invalid pop) error = %v, want nil", err)
	}
	opened.Inner.Body = body
	if err := wire.SignInner(fx.requesterPriv, &opened.Inner); err != nil {
		t.Fatalf("wire.SignInner(invalid pop) error = %v, want nil", err)
	}

	_, _, err = AuthorityHandleJoinRequest(
		store,
		fx.networkID,
		opened.Outer.MsgID,
		opened,
		fx.authorityPriv,
		fx.invite.AuthorityX25519Pub,
		fx.enrollResponse,
	)
	if !errors.Is(err, wire.ErrInvalidSignature) {
		t.Fatalf("AuthorityHandleJoinRequest(invalid pop) error = %v, want %v", err, wire.ErrInvalidSignature)
	}
}

func TestJoinerPersistBootstrapWritesGroupedState(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	store, err := persist.Open(filepath.Join(t.TempDir(), "state"))
	if err != nil {
		t.Fatalf("persist.Open() error = %v, want nil", err)
	}

	if err := JoinerPersistBootstrap(store, fx.enrollResponse); err != nil {
		t.Fatalf("JoinerPersistBootstrap() error = %v, want nil", err)
	}

	memberCredential, err := store.LoadSelfMemberCredential(fx.networkID)
	if err != nil {
		t.Fatalf("LoadSelfMemberCredential() error = %v, want nil", err)
	}
	if len(memberCredential) == 0 {
		t.Fatalf("LoadSelfMemberCredential() = empty, want non-empty")
	}

	mailboxSecret, err := store.LoadMailboxSecret(fx.networkID)
	if err != nil {
		t.Fatalf("LoadMailboxSecret() error = %v, want nil", err)
	}
	if !bytes.Equal(mailboxSecret, fx.enrollResponse.MailboxSecret) {
		t.Fatalf("LoadMailboxSecret() mismatch")
	}
}

func TestJoinerPersistBootstrapRejectsNilStore(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	err := JoinerPersistBootstrap(nil, fx.enrollResponse)
	var enrollErr *Error
	if !errors.As(err, &enrollErr) {
		t.Fatalf("JoinerPersistBootstrap(nil store) error = %v, want enroll.Error", err)
	}
	if enrollErr.Stage != StagePersistence {
		t.Fatalf("JoinerPersistBootstrap(nil store) stage = %q, want %q", enrollErr.Stage, StagePersistence)
	}
}

func TestJoinerPersistBootstrapRejectsInvalidResponse(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	store, err := persist.Open(filepath.Join(t.TempDir(), "state"))
	if err != nil {
		t.Fatalf("persist.Open() error = %v, want nil", err)
	}

	invalid := fx.enrollResponse
	invalid.MailboxSecret = []byte("short")
	err = JoinerPersistBootstrap(store, invalid)
	var enrollErr *Error
	if !errors.As(err, &enrollErr) {
		t.Fatalf("JoinerPersistBootstrap(invalid response) error = %v, want enroll.Error", err)
	}
	if enrollErr.Stage != StagePersistence {
		t.Fatalf("JoinerPersistBootstrap(invalid response) stage = %q, want %q", enrollErr.Stage, StagePersistence)
	}
	if !errors.Is(err, ErrInvalidJoinRequest) {
		t.Fatalf("JoinerPersistBootstrap(invalid response) error = %v, want %v", err, ErrInvalidJoinRequest)
	}
	if _, err := store.LoadSelfMemberCredential(fx.networkID); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("LoadSelfMemberCredential(after invalid response) error = %v, want %v", err, os.ErrNotExist)
	}
}

func mustOpenedJoinRequest(t *testing.T, fx enrollFixture) wire.OpenedMessage {
	t.Helper()

	body, err := fx.joinRequest.MarshalBinary()
	if err != nil {
		t.Fatalf("JoinRequest.MarshalBinary() error = %v, want nil", err)
	}
	requesterPeerID, err := fx.joinRequest.PeerID()
	if err != nil {
		t.Fatalf("JoinRequest.PeerID() error = %v, want nil", err)
	}
	authorityPeerID, err := wire.PeerIDFromEd25519Pub(fx.authorityPub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(authority) error = %v, want nil", err)
	}

	inner := wire.InnerMessage{
		DstPeerID:       authorityPeerID,
		MsgID:           mustMsgID(t, "MFRGGZDFMZTWQ2LKNNWG23TPOI"),
		CreatedAtUnixMs: fx.joinRequest.CreatedAtUnixMs,
		ExpiresAtUnixMs: fx.joinRequest.ExpiresAtUnixMs,
		SenderPeerID:    requesterPeerID,
		SenderEd25519:   append([]byte(nil), fx.requesterPub...),
		Kind:            wire.KindJoinRequest,
		Body:            body,
	}
	if err := wire.SignInner(fx.requesterPriv, &inner); err != nil {
		t.Fatalf("wire.SignInner() error = %v, want nil", err)
	}

	return wire.OpenedMessage{
		Outer: wire.OuterHeader{
			Version:         wire.OuterVersionV1,
			DstPeerID:       authorityPeerID,
			SrcPeerID:       requesterPeerID,
			MsgID:           inner.MsgID,
			ExpiresAtUnixMs: inner.ExpiresAtUnixMs,
			Scheme:          wire.SchemePeerE2EV1,
			Ciphertext:      []byte("stub"),
		},
		Inner: inner,
	}
}

func mustOtherJoinRequestSigner(t *testing.T) ed25519.PrivateKey {
	t.Helper()

	return ed25519.NewKeyFromSeed(bytes.Repeat([]byte{0x55}, ed25519.SeedSize))
}
