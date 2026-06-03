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
	"crypto/ecdh"
	"crypto/ed25519"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"strings"
	"testing"

	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

func TestInviteCapabilityRoundTrip(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	code, err := fx.invite.InviteCode()
	if err != nil {
		t.Fatalf("InviteCode() error = %v, want nil", err)
	}

	parsed, err := ParseInviteCode(code)
	if err != nil {
		t.Fatalf("ParseInviteCode() error = %v, want nil", err)
	}
	if diff := diffInviteCapability(fx.invite, parsed); diff != "" {
		t.Fatalf("ParseInviteCode() mismatch (-want +got):\n%s", diff)
	}
	if err := VerifyInviteCapability(parsed); err != nil {
		t.Fatalf("VerifyInviteCapability() error = %v, want nil", err)
	}
}

func TestVerifyInviteCapabilityRejectsTampering(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	tampered := fx.invite
	tampered.Signature = append([]byte(nil), fx.invite.Signature...)
	tampered.Signature[0] ^= 0xff
	if err := VerifyInviteCapability(tampered); !errors.Is(err, wire.ErrInvalidSignature) {
		t.Fatalf("VerifyInviteCapability(tampered) error = %v, want %v", err, wire.ErrInvalidSignature)
	}
}

func TestSignInviteCapabilityRejectsSignerMismatch(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	otherPriv := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{0x66}, ed25519.SeedSize))
	otherPub := otherPriv.Public().(ed25519.PublicKey)

	invite := fx.invite
	invite.AuthorityEd25519Pub = append([]byte(nil), otherPub...)
	invite.Signature = nil
	if err := SignInviteCapability(fx.authorityPriv, &invite); !errors.Is(err, ErrInvalidInviteCode) {
		t.Fatalf("SignInviteCapability(signer mismatch) error = %v, want %v", err, ErrInvalidInviteCode)
	}
}

func TestParseInviteCodeRejectsInvalidInput(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	code, err := fx.invite.InviteCode()
	if err != nil {
		t.Fatalf("InviteCode() error = %v, want nil", err)
	}

	tests := []struct {
		name  string
		code  string
		check func(error) bool
	}{
		{
			name: "missing_prefix",
			code: strings.TrimPrefix(code, MPINV1Prefix),
			check: func(err error) bool {
				return errors.Is(err, ErrInvalidInviteCode)
			},
		},
		{
			name: "invalid_payload",
			code: MPINV1Prefix + "***",
			check: func(err error) bool {
				return errors.Is(err, ErrInvalidInviteCode)
			},
		},
	}

	unsignedPayload, err := marshalInviteCapability(fx.invite, false)
	if err != nil {
		t.Fatalf("marshalInviteCapability(unsigned) error = %v, want nil", err)
	}
	tests = append(tests, struct {
		name  string
		code  string
		check func(error) bool
	}{
		name: "missing_signature",
		code: MPINV1Prefix + base64.RawURLEncoding.EncodeToString(unsignedPayload),
		check: func(err error) bool {
			return errors.Is(err, ErrInvalidInviteCode)
		},
	})

	for _, tc := range tests {
		tc := tc
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()

			_, err := ParseInviteCode(tc.code)
			if err == nil || !tc.check(err) {
				t.Fatalf("ParseInviteCode(%q) error = %v, want invalid invite code", tc.name, err)
			}
		})
	}
}

func TestJoinRequestPoPVerification(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	data, err := fx.joinRequest.MarshalBinary()
	if err != nil {
		t.Fatalf("JoinRequest.MarshalBinary() error = %v, want nil", err)
	}

	decoded, err := UnmarshalJoinRequest(data)
	if err != nil {
		t.Fatalf("UnmarshalJoinRequest() error = %v, want nil", err)
	}
	if diff := diffJoinRequest(fx.joinRequest, decoded); diff != "" {
		t.Fatalf("UnmarshalJoinRequest() mismatch (-want +got):\n%s", diff)
	}
	if err := VerifyJoinRequest(decoded); err != nil {
		t.Fatalf("VerifyJoinRequest() error = %v, want nil", err)
	}

	tampered := decoded
	tampered.DeviceName = "other"
	if err := VerifyJoinRequest(tampered); !errors.Is(err, wire.ErrInvalidSignature) {
		t.Fatalf("VerifyJoinRequest(tampered) error = %v, want %v", err, wire.ErrInvalidSignature)
	}
}

func TestMemberCredentialVerification(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	data, err := fx.memberCredential.MarshalBinary()
	if err != nil {
		t.Fatalf("MemberCredential.MarshalBinary() error = %v, want nil", err)
	}

	decoded, err := UnmarshalMemberCredential(data)
	if err != nil {
		t.Fatalf("UnmarshalMemberCredential() error = %v, want nil", err)
	}
	if diff := diffMemberCredential(fx.memberCredential, decoded); diff != "" {
		t.Fatalf("UnmarshalMemberCredential() mismatch (-want +got):\n%s", diff)
	}
	if err := VerifyMemberCredential(decoded, fx.authorityPub); err != nil {
		t.Fatalf("VerifyMemberCredential() error = %v, want nil", err)
	}
}

func TestMemberCredentialVerificationRejectsWrongIssuer(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	otherPriv := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{0x77}, ed25519.SeedSize))
	otherPub := otherPriv.Public().(ed25519.PublicKey)
	if err := VerifyMemberCredential(fx.memberCredential, otherPub); !errors.Is(err, wire.ErrInvalidSignature) {
		t.Fatalf("VerifyMemberCredential(wrong issuer) error = %v, want %v", err, wire.ErrInvalidSignature)
	}
}

func TestMemberCredentialVerificationRejectsTampering(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	tampered := fx.memberCredential
	tampered.Role = "admin"
	if err := VerifyMemberCredential(tampered, fx.authorityPub); !errors.Is(err, wire.ErrInvalidSignature) {
		t.Fatalf("VerifyMemberCredential(tampered) error = %v, want %v", err, wire.ErrInvalidSignature)
	}
}

func TestEnrollResponseRoundTripAndPersistHandoff(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	data, err := fx.enrollResponse.MarshalBinary()
	if err != nil {
		t.Fatalf("EnrollResponse.MarshalBinary() error = %v, want nil", err)
	}

	decoded, err := UnmarshalEnrollResponse(data)
	if err != nil {
		t.Fatalf("UnmarshalEnrollResponse() error = %v, want nil", err)
	}
	if diff := diffEnrollResponse(fx.enrollResponse, decoded); diff != "" {
		t.Fatalf("UnmarshalEnrollResponse() mismatch (-want +got):\n%s", diff)
	}

	joined, err := decoded.JoinedBootstrap()
	if err != nil {
		t.Fatalf("JoinedBootstrap() error = %v, want nil", err)
	}
	if joined.NetworkID != fx.networkID {
		t.Fatalf("JoinedBootstrap().NetworkID = %q, want %q", joined.NetworkID, fx.networkID)
	}
	if !bytes.Equal(joined.MailboxSecret, fx.enrollResponse.MailboxSecret) {
		t.Fatalf("JoinedBootstrap().MailboxSecret mismatch")
	}
	if strings.Join(joined.RuntimeBroker.StunServers, ",") != strings.Join(fx.enrollResponse.RuntimeBroker.StunServers, ",") {
		t.Fatalf("JoinedBootstrap().RuntimeBroker.StunServers = %#v, want %#v", joined.RuntimeBroker.StunServers, fx.enrollResponse.RuntimeBroker.StunServers)
	}
}

func TestEnrollResponseValidationRejectsInvalidFields(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	mismatchedPeerID := mustPeerID(t, fx.authorityPub)
	duplicateEntry := RosterEntry{
		PeerID:           fx.enrollResponse.RosterSnapshot.Entries[0].PeerID,
		MemberCredential: fx.memberCredential,
		DeviceName:       "beta",
		Platform:         "windows",
	}

	tests := []struct {
		name     string
		response EnrollResponse
		wantErr  error
	}{
		{
			name: "invalid_mailbox_secret_length",
			response: EnrollResponse{
				SelfMemberCredential: fx.enrollResponse.SelfMemberCredential,
				MailboxSecret:        []byte("short"),
				RuntimeBroker:        fx.enrollResponse.RuntimeBroker,
				RosterSnapshot:       fx.enrollResponse.RosterSnapshot,
			},
			wantErr: ErrInvalidJoinRequest,
		},
		{
			name: "empty_runtime_broker",
			response: EnrollResponse{
				SelfMemberCredential: fx.enrollResponse.SelfMemberCredential,
				MailboxSecret:        append([]byte(nil), fx.enrollResponse.MailboxSecret...),
				RuntimeBroker:        RuntimeBroker{},
				RosterSnapshot:       fx.enrollResponse.RosterSnapshot,
			},
			wantErr: ErrInvalidBrokerEndpoint,
		},
		{
			name: "duplicate_roster_peer_id",
			response: EnrollResponse{
				SelfMemberCredential: fx.enrollResponse.SelfMemberCredential,
				MailboxSecret:        append([]byte(nil), fx.enrollResponse.MailboxSecret...),
				RuntimeBroker:        fx.enrollResponse.RuntimeBroker,
				RosterSnapshot: RosterSnapshot{
					Entries: []RosterEntry{
						fx.enrollResponse.RosterSnapshot.Entries[0],
						duplicateEntry,
					},
				},
			},
			wantErr: ErrInvalidRosterSnapshot,
		},
		{
			name: "roster_peer_id_mismatch",
			response: EnrollResponse{
				SelfMemberCredential: fx.enrollResponse.SelfMemberCredential,
				MailboxSecret:        append([]byte(nil), fx.enrollResponse.MailboxSecret...),
				RuntimeBroker:        fx.enrollResponse.RuntimeBroker,
				RosterSnapshot: RosterSnapshot{
					Entries: []RosterEntry{
						{
							PeerID:           mismatchedPeerID,
							MemberCredential: fx.memberCredential,
							DeviceName:       "alpha",
							Platform:         "linux",
						},
					},
				},
			},
			wantErr: ErrInvalidRosterSnapshot,
		},
	}

	for _, tc := range tests {
		tc := tc
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()

			if _, err := tc.response.MarshalBinary(); !errors.Is(err, tc.wantErr) {
				t.Fatalf("EnrollResponse.MarshalBinary(%s) error = %v, want %v", tc.name, err, tc.wantErr)
			}
		})
	}
}

func TestInviteCodeGolden(t *testing.T) {
	t.Parallel()

	fx := mustEnrollFixture(t)
	data, err := fx.invite.MarshalBinary()
	if err != nil {
		t.Fatalf("InviteCapability.MarshalBinary() error = %v, want nil", err)
	}
	if len(data) == 0 {
		t.Fatalf("InviteCapability.MarshalBinary() produced empty output")
	}
	if got := hex.EncodeToString(data); strings.TrimSpace(got) == "" {
		t.Fatalf("InviteCapability.MarshalBinary() hex = empty")
	}
}

type enrollFixture struct {
	networkID           string
	authorityPriv       ed25519.PrivateKey
	authorityPub        ed25519.PublicKey
	authorityX25519Priv []byte
	authorityX25519Pub  []byte
	requesterPriv       ed25519.PrivateKey
	requesterPub        ed25519.PublicKey
	requesterX25519Priv []byte
	requesterX25519Pub  []byte
	invite              InviteCapability
	joinRequest         JoinRequest
	memberCredential    MemberCredential
	enrollResponse      EnrollResponse
}

func mustEnrollFixture(t *testing.T) enrollFixture {
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
	invite := InviteCapability{
		NetworkIDBytes:      bytes.Repeat([]byte{0x5a}, wire.RawIDLen),
		AuthorityEd25519Pub: append([]byte(nil), authorityPub...),
		AuthorityX25519Pub:  append([]byte(nil), authorityX25519Pub...),
		BrokerEndpoint:      "broker.example.net:1883",
		JoinTopic:           "mp/v1/join/demo",
		InviteID:            mustMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA"),
		NotAfterUnixMs:      1_717_000_030_000,
	}
	if err := SignInviteCapability(authorityPriv, &invite); err != nil {
		t.Fatalf("SignInviteCapability() error = %v, want nil", err)
	}

	joinRequest := JoinRequest{
		InviteID:            invite.InviteID,
		RequesterEd25519Pub: append([]byte(nil), requesterPub...),
		RequesterX25519Pub:  append([]byte(nil), requesterX25519Pub...),
		ReplyTopic:          "mp/v1/reply/demo",
		DeviceName:          "alpha",
		Platform:            "linux",
		CreatedAtUnixMs:     1_717_000_000_000,
		ExpiresAtUnixMs:     1_717_000_030_000,
	}
	if err := SignJoinRequest(requesterPriv, &joinRequest); err != nil {
		t.Fatalf("SignJoinRequest() error = %v, want nil", err)
	}

	memberCredential := MemberCredential{
		NetworkID:         networkID,
		SubjectEd25519Pub: append([]byte(nil), requesterPub...),
		SubjectX25519Pub:  append([]byte(nil), requesterX25519Pub...),
		Role:              "member",
		NotBeforeUnixMs:   1_717_000_000_000,
		NotAfterUnixMs:    1_717_000_060_000,
		IssuerKeyID:       "authority-01",
	}
	if err := SignMemberCredential(authorityPriv, &memberCredential); err != nil {
		t.Fatalf("SignMemberCredential() error = %v, want nil", err)
	}

	enrollResponse := EnrollResponse{
		SelfMemberCredential: memberCredential,
		MailboxSecret:        bytes.Repeat([]byte{0x33}, mailboxSecretSize),
		RuntimeBroker: RuntimeBroker{
			Endpoint:    "broker.example.net:1883",
			StunServers: []string{"stun1.example.net:3478", "stun2.example.net:3478"},
		},
		RosterSnapshot: RosterSnapshot{
			Entries: []RosterEntry{
				{
					PeerID:           mustPeerID(t, requesterPub),
					MemberCredential: memberCredential,
					DeviceName:       "alpha",
					Platform:         "linux",
				},
			},
		},
	}

	return enrollFixture{
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
		memberCredential:    memberCredential,
		enrollResponse:      enrollResponse,
	}
}

func mustMsgID(t *testing.T, value string) string {
	t.Helper()

	msgID, err := wire.CanonicalizeMsgID(value)
	if err != nil {
		t.Fatalf("CanonicalizeMsgID(%q) error = %v, want nil", value, err)
	}
	return msgID
}

func mustPeerID(t *testing.T, pub ed25519.PublicKey) string {
	t.Helper()

	peerID, err := wire.PeerIDFromEd25519Pub(pub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub() error = %v, want nil", err)
	}
	return peerID
}

func diffInviteCapability(want, got InviteCapability) string {
	switch {
	case !bytes.Equal(want.NetworkIDBytes, got.NetworkIDBytes):
		return "network_id_bytes mismatch"
	case !bytes.Equal(want.AuthorityEd25519Pub, got.AuthorityEd25519Pub):
		return "authority_ed25519_pub mismatch"
	case !bytes.Equal(want.AuthorityX25519Pub, got.AuthorityX25519Pub):
		return "authority_x25519_pub mismatch"
	case want.BrokerEndpoint != got.BrokerEndpoint:
		return "broker_endpoint mismatch"
	case want.JoinTopic != got.JoinTopic:
		return "join_topic mismatch"
	case want.InviteID != got.InviteID:
		return "invite_id mismatch"
	case want.NotAfterUnixMs != got.NotAfterUnixMs:
		return "not_after mismatch"
	default:
		return ""
	}
}

func diffJoinRequest(want, got JoinRequest) string {
	switch {
	case want.InviteID != got.InviteID:
		return "invite_id mismatch"
	case !bytes.Equal(want.RequesterEd25519Pub, got.RequesterEd25519Pub):
		return "requester_ed25519_pub mismatch"
	case !bytes.Equal(want.RequesterX25519Pub, got.RequesterX25519Pub):
		return "requester_x25519_pub mismatch"
	case want.ReplyTopic != got.ReplyTopic:
		return "reply_topic mismatch"
	case want.DeviceName != got.DeviceName:
		return "device_name mismatch"
	case want.Platform != got.Platform:
		return "platform mismatch"
	case want.CreatedAtUnixMs != got.CreatedAtUnixMs:
		return "created_at mismatch"
	case want.ExpiresAtUnixMs != got.ExpiresAtUnixMs:
		return "expires_at mismatch"
	default:
		return ""
	}
}

func diffMemberCredential(want, got MemberCredential) string {
	switch {
	case want.NetworkID != got.NetworkID:
		return "network_id mismatch"
	case !bytes.Equal(want.SubjectEd25519Pub, got.SubjectEd25519Pub):
		return "subject_ed25519_pub mismatch"
	case !bytes.Equal(want.SubjectX25519Pub, got.SubjectX25519Pub):
		return "subject_x25519_pub mismatch"
	case want.Role != got.Role:
		return "role mismatch"
	case want.NotBeforeUnixMs != got.NotBeforeUnixMs:
		return "not_before mismatch"
	case want.NotAfterUnixMs != got.NotAfterUnixMs:
		return "not_after mismatch"
	case want.IssuerKeyID != got.IssuerKeyID:
		return "issuer_key_id mismatch"
	default:
		return ""
	}
}

func diffEnrollResponse(want, got EnrollResponse) string {
	if diff := diffMemberCredential(want.SelfMemberCredential, got.SelfMemberCredential); diff != "" {
		return diff
	}
	if !bytes.Equal(want.MailboxSecret, got.MailboxSecret) {
		return "mailbox_secret mismatch"
	}
	if want.RuntimeBroker.Endpoint != got.RuntimeBroker.Endpoint {
		return "runtime_broker mismatch"
	}
	if strings.Join(want.RuntimeBroker.StunServers, ",") != strings.Join(got.RuntimeBroker.StunServers, ",") {
		return "stun_servers mismatch"
	}
	if len(want.RosterSnapshot.Entries) != len(got.RosterSnapshot.Entries) {
		return "roster entry count mismatch"
	}
	return ""
}

func x25519Public(rawPrivate []byte) ([]byte, error) {
	priv, err := ecdh.X25519().NewPrivateKey(rawPrivate)
	if err != nil {
		return nil, err
	}
	return priv.PublicKey().Bytes(), nil
}
