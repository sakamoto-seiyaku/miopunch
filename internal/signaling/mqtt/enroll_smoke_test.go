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

package mqtt

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"errors"
	"os"
	"testing"
	"time"

	"github.com/256dpi/gomqtt/broker"
	"github.com/256dpi/gomqtt/transport"

	"github.com/miopunch/miopunch/internal/pocv1/enroll"
	"github.com/miopunch/miopunch/internal/pocv1/peere2e"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

func TestEnrollBootstrapSmoke(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	brokerURL, cleanup := launchEnrollSmokeBroker(t)
	defer cleanup()

	fx := mustEnrollSmokeFixture(t)
	topic := fx.invite.JoinTopic

	authoritySession, err := OpenPeerMessageSession(ctx, PeerMessageConfig{
		BrokerURL:       brokerURL,
		SubscribeTopics: []string{topic},
	})
	if err != nil {
		t.Fatalf("OpenPeerMessageSession(authority) error = %v, want nil", err)
	}
	defer authoritySession.Close()

	joinerSession, err := OpenPeerMessageSession(ctx, PeerMessageConfig{
		BrokerURL:       brokerURL,
		SubscribeTopics: []string{fx.joinRequest.ReplyTopic},
	})
	if err != nil {
		t.Fatalf("OpenPeerMessageSession(joiner) error = %v, want nil", err)
	}
	defer joinerSession.Close()

	joinReqMsg, err := fx.joinRequestMessage()
	if err != nil {
		t.Fatalf("joinRequestMessage() error = %v, want nil", err)
	}
	sealedJoinReq, err := peere2e.Seal(joinReqMsg.Outer, joinReqMsg.Inner, fx.invite.AuthorityX25519Pub, peere2e.SealOptions{})
	if err != nil {
		t.Fatalf("peere2e.Seal(join_request) error = %v, want nil", err)
	}
	if err := publishPayload(ctx, joinerSession, topic, mustPayload(t, sealedJoinReq)); err != nil {
		t.Fatalf("publish join_request error = %v, want nil", err)
	}

	opened, err := authoritySession.WaitOpened(ctx, fx.authorityX25519Priv, peere2e.OpenOptions{})
	if err != nil {
		t.Fatalf("WaitOpened(authority) error = %v, want nil", err)
	}
	response, hit, err := enroll.AuthorityHandleJoinRequest(
		fx.authorityStore,
		fx.networkID,
		opened.Outer.MsgID,
		wire.OpenedMessage{Outer: opened.Outer, Inner: opened.Inner},
		fx.authorityPriv,
		fx.authorityX25519Pub,
		fx.response,
	)
	if err != nil {
		t.Fatalf("AuthorityHandleJoinRequest() error = %v, want nil", err)
	}
	if hit {
		t.Fatalf("AuthorityHandleJoinRequest() hit = true, want false")
	}
	if len(response) == 0 {
		t.Fatalf("AuthorityHandleJoinRequest() = empty response")
	}

	if err := publishPayload(ctx, authoritySession, fx.joinRequest.ReplyTopic, response); err != nil {
		t.Fatalf("publish enroll response error = %v, want nil", err)
	}

	openedResp, err := joinerSession.WaitOpened(ctx, fx.requesterX25519Priv, peere2e.OpenOptions{})
	if err != nil {
		t.Fatalf("WaitOpened(joiner) error = %v, want nil", err)
	}
	enrollResp, err := enroll.UnmarshalEnrollResponse(openedResp.Inner.Body)
	if err != nil {
		t.Fatalf("UnmarshalEnrollResponse() error = %v, want nil", err)
	}
	if err := enroll.JoinerPersistBootstrap(fx.joinerStore, enrollResp); err != nil {
		t.Fatalf("JoinerPersistBootstrap() error = %v, want nil", err)
	}

	roster, err := fx.joinerStore.LoadRosterSnapshot(fx.networkID)
	if err != nil {
		t.Fatalf("LoadRosterSnapshot() error = %v, want nil", err)
	}
	if len(roster.Entries) != 1 {
		t.Fatalf("LoadRosterSnapshot().Entries = %d, want 1", len(roster.Entries))
	}
	if _, err := fx.authorityStore.LoadEnrollHandledRequest(fx.networkID, opened.Outer.MsgID); err != nil {
		t.Fatalf("LoadEnrollHandledRequest(authority) error = %v, want nil", err)
	}
}

func TestEnrollBootstrapReplayCacheSmoke(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	brokerURL, cleanup := launchEnrollSmokeBroker(t)
	defer cleanup()

	fx := mustEnrollSmokeFixture(t)
	topic := fx.invite.JoinTopic

	authoritySession, err := OpenPeerMessageSession(ctx, PeerMessageConfig{
		BrokerURL:       brokerURL,
		SubscribeTopics: []string{topic},
	})
	if err != nil {
		t.Fatalf("OpenPeerMessageSession(authority) error = %v, want nil", err)
	}
	defer authoritySession.Close()

	joinerSession, err := OpenPeerMessageSession(ctx, PeerMessageConfig{
		BrokerURL:       brokerURL,
		SubscribeTopics: []string{fx.joinRequest.ReplyTopic},
	})
	if err != nil {
		t.Fatalf("OpenPeerMessageSession(joiner) error = %v, want nil", err)
	}
	defer joinerSession.Close()

	joinReqMsg, err := fx.joinRequestMessage()
	if err != nil {
		t.Fatalf("joinRequestMessage() error = %v, want nil", err)
	}
	sealedJoinReq, err := peere2e.Seal(joinReqMsg.Outer, joinReqMsg.Inner, fx.invite.AuthorityX25519Pub, peere2e.SealOptions{})
	if err != nil {
		t.Fatalf("peere2e.Seal(join_request) error = %v, want nil", err)
	}
	payload := mustPayload(t, sealedJoinReq)

	if err := publishPayload(ctx, joinerSession, topic, payload); err != nil {
		t.Fatalf("publish first join_request error = %v, want nil", err)
	}

	opened, err := authoritySession.WaitOpened(ctx, fx.authorityX25519Priv, peere2e.OpenOptions{})
	if err != nil {
		t.Fatalf("WaitOpened(authority first) error = %v, want nil", err)
	}
	firstResponse, hit, err := enroll.AuthorityHandleJoinRequest(
		fx.authorityStore,
		fx.networkID,
		opened.Outer.MsgID,
		wire.OpenedMessage{Outer: opened.Outer, Inner: opened.Inner},
		fx.authorityPriv,
		fx.authorityX25519Pub,
		fx.response,
	)
	if err != nil {
		t.Fatalf("AuthorityHandleJoinRequest(first) error = %v, want nil", err)
	}
	if hit {
		t.Fatalf("AuthorityHandleJoinRequest(first) hit = true, want false")
	}

	if err := publishPayload(ctx, joinerSession, topic, payload); err != nil {
		t.Fatalf("publish replayed join_request error = %v, want nil", err)
	}

	replayed, err := authoritySession.WaitOpened(ctx, fx.authorityX25519Priv, peere2e.OpenOptions{})
	if err != nil {
		t.Fatalf("WaitOpened(authority replay) error = %v, want nil", err)
	}
	secondResponse, hit, err := enroll.AuthorityHandleJoinRequest(
		fx.authorityStore,
		fx.networkID,
		replayed.Outer.MsgID,
		wire.OpenedMessage{Outer: replayed.Outer, Inner: replayed.Inner},
		fx.authorityPriv,
		fx.authorityX25519Pub,
		fx.response,
	)
	if err != nil {
		t.Fatalf("AuthorityHandleJoinRequest(replay) error = %v, want nil", err)
	}
	if !hit {
		t.Fatalf("AuthorityHandleJoinRequest(replay) hit = false, want true")
	}
	if !bytes.Equal(firstResponse, secondResponse) {
		t.Fatalf("AuthorityHandleJoinRequest(replay) response mismatch")
	}

	if _, err := fx.joinerStore.LoadSelfMemberCredential(fx.networkID); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("LoadSelfMemberCredential(before persist) error = %v, want %v", err, os.ErrNotExist)
	}
}

func launchEnrollSmokeBroker(t *testing.T) (string, func()) {
	t.Helper()

	server, err := transport.Launch("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("transport.Launch() error = %v, want nil", err)
	}
	backend := broker.NewMemoryBackend()
	engine := broker.NewEngine(backend)
	engine.Accept(server)
	return "tcp://" + server.Addr().String(), func() {
		_ = server.Close()
		engine.Close()
	}
}

type enrollSmokeFixture struct {
	networkID           string
	authorityPriv       ed25519.PrivateKey
	authorityPub        ed25519.PublicKey
	authorityX25519Priv []byte
	authorityX25519Pub  []byte
	requesterPriv       ed25519.PrivateKey
	requesterPub        ed25519.PublicKey
	requesterX25519Priv []byte
	requesterX25519Pub  []byte
	invite              enroll.InviteCapability
	joinRequest         enroll.JoinRequest
	response            enroll.EnrollResponse
	authorityStore      *persist.Store
	joinerStore         *persist.Store
}

func mustEnrollSmokeFixture(t *testing.T) enrollSmokeFixture {
	t.Helper()

	authoritySeed := bytes.Repeat([]byte{0x11}, ed25519.SeedSize)
	authorityPriv := ed25519.NewKeyFromSeed(authoritySeed)
	authorityPub := authorityPriv.Public().(ed25519.PublicKey)
	authorityX25519Priv := bytes.Repeat([]byte{0x33}, 32)
	authorityX25519Pub, err := x25519Public(authorityX25519Priv)
	if err != nil {
		t.Fatalf("x25519Public(authority) error = %v, want nil", err)
	}

	requesterSeed := bytes.Repeat([]byte{0x22}, ed25519.SeedSize)
	requesterPriv := ed25519.NewKeyFromSeed(requesterSeed)
	requesterPub := requesterPriv.Public().(ed25519.PublicKey)
	requesterX25519Priv := bytes.Repeat([]byte{0x44}, 32)
	requesterX25519Pub, err := x25519Public(requesterX25519Priv)
	if err != nil {
		t.Fatalf("x25519Public(requester) error = %v, want nil", err)
	}

	networkID, err := wire.EncodeNetworkID(bytes.Repeat([]byte{0x5a}, wire.RawIDLen))
	if err != nil {
		t.Fatalf("EncodeNetworkID() error = %v, want nil", err)
	}

	invite := enroll.InviteCapability{
		NetworkIDBytes:      bytes.Repeat([]byte{0x5a}, wire.RawIDLen),
		AuthorityEd25519Pub: append([]byte(nil), authorityPub...),
		AuthorityX25519Pub:  append([]byte(nil), authorityX25519Pub...),
		BrokerEndpoint:      "broker.example.net:1883",
		JoinTopic:           "mp/v1/join/demo",
		InviteID:            mustEnrollMsgID("JBSWY3DPEHPK3PXPJBSWY3DPAA"),
		NotAfterUnixMs:      1_717_000_030_000,
	}
	if err := enroll.SignInviteCapability(authorityPriv, &invite); err != nil {
		t.Fatalf("SignInviteCapability() error = %v, want nil", err)
	}

	joinRequest := enroll.JoinRequest{
		InviteID:            invite.InviteID,
		RequesterEd25519Pub: append([]byte(nil), requesterPub...),
		RequesterX25519Pub:  append([]byte(nil), requesterX25519Pub...),
		ReplyTopic:          "mp/v1/reply/demo",
		DeviceName:          "alpha",
		Platform:            "linux",
		CreatedAtUnixMs:     1_717_000_000_000,
		ExpiresAtUnixMs:     1_717_000_030_000,
	}
	if err := enroll.SignJoinRequest(requesterPriv, &joinRequest); err != nil {
		t.Fatalf("SignJoinRequest() error = %v, want nil", err)
	}

	memberCredential := enroll.MemberCredential{
		NetworkID:         networkID,
		SubjectEd25519Pub: append([]byte(nil), requesterPub...),
		SubjectX25519Pub:  append([]byte(nil), requesterX25519Pub...),
		Role:              "member",
		NotBeforeUnixMs:   1_717_000_000_000,
		NotAfterUnixMs:    1_717_000_060_000,
		IssuerKeyID:       "authority-01",
	}
	if err := enroll.SignMemberCredential(authorityPriv, &memberCredential); err != nil {
		t.Fatalf("SignMemberCredential() error = %v, want nil", err)
	}

	response := enroll.EnrollResponse{
		SelfMemberCredential: memberCredential,
		MailboxSecret:        bytes.Repeat([]byte{0x33}, 32),
		RuntimeBroker:        enroll.RuntimeBroker{Endpoint: "broker.example.net:1883"},
		RosterSnapshot: enroll.RosterSnapshot{
			Entries: []enroll.RosterEntry{
				{
					PeerID:           mustEnrollPeerID(requesterPub),
					MemberCredential: memberCredential,
					DeviceName:       "alpha",
					Platform:         "linux",
				},
			},
		},
	}

	authorityStore, err := persist.Open(t.TempDir())
	if err != nil {
		t.Fatalf("persist.Open(authority) error = %v, want nil", err)
	}
	joinerStore, err := persist.Open(t.TempDir())
	if err != nil {
		t.Fatalf("persist.Open(joiner) error = %v, want nil", err)
	}

	return enrollSmokeFixture{
		networkID:           networkID,
		authorityPriv:       authorityPriv,
		authorityPub:        authorityPub,
		authorityX25519Priv: authorityX25519Priv,
		authorityX25519Pub:  authorityX25519Pub,
		requesterPriv:       requesterPriv,
		requesterPub:        requesterPub,
		requesterX25519Priv: requesterX25519Priv,
		requesterX25519Pub:  requesterX25519Pub,
		invite:              invite,
		joinRequest:         joinRequest,
		response:            response,
		authorityStore:      authorityStore,
		joinerStore:         joinerStore,
	}
}

func (f enrollSmokeFixture) joinRequestMessage() (wire.OpenedMessage, error) {
	body, err := f.joinRequest.MarshalBinary()
	if err != nil {
		return wire.OpenedMessage{}, err
	}
	requesterPeerID, err := f.joinRequest.PeerID()
	if err != nil {
		return wire.OpenedMessage{}, err
	}
	authorityPeerID, err := wire.PeerIDFromEd25519Pub(f.authorityPub)
	if err != nil {
		return wire.OpenedMessage{}, err
	}
	inner := wire.InnerMessage{
		DstPeerID:       authorityPeerID,
		MsgID:           mustEnrollMsgID("MFRGGZDFMZTWQ2LKNNWG23TPOI"),
		CreatedAtUnixMs: f.joinRequest.CreatedAtUnixMs,
		ExpiresAtUnixMs: f.joinRequest.ExpiresAtUnixMs,
		SenderPeerID:    requesterPeerID,
		SenderEd25519:   append([]byte(nil), f.requesterPub...),
		Kind:            wire.KindJoinRequest,
		Body:            body,
	}
	if err := wire.SignInner(f.requesterPriv, &inner); err != nil {
		return wire.OpenedMessage{}, err
	}
	return wire.OpenedMessage{
		Outer: wire.OuterHeader{
			Version:         wire.OuterVersionV1,
			DstPeerID:       authorityPeerID,
			SrcPeerID:       requesterPeerID,
			MsgID:           inner.MsgID,
			ExpiresAtUnixMs: inner.ExpiresAtUnixMs,
			Scheme:          wire.SchemePeerE2EV1,
		},
		Inner: inner,
	}, nil
}

func mustPayload(t *testing.T, outer wire.OuterHeader) []byte {
	t.Helper()

	payload, err := outer.MarshalBinary()
	if err != nil {
		t.Fatalf("OuterHeader.MarshalBinary() error = %v, want nil", err)
	}
	return payload
}

func publishPayload(ctx context.Context, session *PeerMessageSession, topic string, payload []byte) error {
	f, err := session.c.Publish(topic, payload, 1, false)
	if err != nil {
		return err
	}
	return waitFuture(ctx, f, defaultPeerMessageTimeout)
}

func mustEnrollMsgID(value string) string {
	msgID, err := wire.CanonicalizeMsgID(value)
	if err != nil {
		panic(err)
	}
	return msgID
}

func mustEnrollPeerID(pub ed25519.PublicKey) string {
	peerID, err := wire.PeerIDFromEd25519Pub(pub)
	if err != nil {
		panic(err)
	}
	return peerID
}
