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
	"encoding/base64"
	"encoding/binary"
	"fmt"
	"strings"

	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

const (
	// MPINV1Prefix is the textual prefix for invite codes.
	MPINV1Prefix = "MPINV1-"

	inviteTranscriptDomain           = "miopunch/poc/v1/enroll/invite_capability"
	joinRequestTranscriptDomain      = "miopunch/poc/v1/enroll/join_request"
	memberCredentialTranscriptDomain = "miopunch/poc/v1/enroll/member_credential"
)

const (
	inviteTagNetworkIDBytes      = 1
	inviteTagAuthorityEd25519Pub = 2
	inviteTagAuthorityX25519Pub  = 3
	inviteTagBrokerEndpoint      = 4
	inviteTagJoinTopic           = 5
	inviteTagInviteID            = 6
	inviteTagNotAfterUnixMs      = 7
	inviteTagSignature           = 8

	joinRequestTagInviteID            = 1
	joinRequestTagRequesterEd25519Pub = 2
	joinRequestTagRequesterX25519Pub  = 3
	joinRequestTagReplyTopic          = 4
	joinRequestTagDeviceName          = 5
	joinRequestTagPlatform            = 6
	joinRequestTagCreatedAtUnixMs     = 7
	joinRequestTagExpiresAtUnixMs     = 8
	joinRequestTagSignature           = 9

	memberCredentialTagNetworkID       = 1
	memberCredentialTagSubjectEd25519  = 2
	memberCredentialTagSubjectX25519   = 3
	memberCredentialTagRole            = 4
	memberCredentialTagNotBeforeUnixMs = 5
	memberCredentialTagNotAfterUnixMs  = 6
	memberCredentialTagIssuerKeyID     = 7
	memberCredentialTagSignature       = 8

	enrollResponseTagSelfMemberCredential = 1
	enrollResponseTagMailboxSecret        = 2
	enrollResponseTagRuntimeBroker        = 3
	enrollResponseTagRosterSnapshot       = 4

	rosterEntryTagPeerID           = 1
	rosterEntryTagMemberCredential = 2
	rosterEntryTagDeviceName       = 3
	rosterEntryTagPlatform         = 4
)

var (
	inviteAllowedTags = []uint64{
		inviteTagNetworkIDBytes,
		inviteTagAuthorityEd25519Pub,
		inviteTagAuthorityX25519Pub,
		inviteTagBrokerEndpoint,
		inviteTagJoinTopic,
		inviteTagInviteID,
		inviteTagNotAfterUnixMs,
		inviteTagSignature,
	}
	joinRequestAllowedTags = []uint64{
		joinRequestTagInviteID,
		joinRequestTagRequesterEd25519Pub,
		joinRequestTagRequesterX25519Pub,
		joinRequestTagReplyTopic,
		joinRequestTagDeviceName,
		joinRequestTagPlatform,
		joinRequestTagCreatedAtUnixMs,
		joinRequestTagExpiresAtUnixMs,
		joinRequestTagSignature,
	}
	memberCredentialAllowedTags = []uint64{
		memberCredentialTagNetworkID,
		memberCredentialTagSubjectEd25519,
		memberCredentialTagSubjectX25519,
		memberCredentialTagRole,
		memberCredentialTagNotBeforeUnixMs,
		memberCredentialTagNotAfterUnixMs,
		memberCredentialTagIssuerKeyID,
		memberCredentialTagSignature,
	}
	enrollResponseAllowedTags = []uint64{
		enrollResponseTagSelfMemberCredential,
		enrollResponseTagMailboxSecret,
		enrollResponseTagRuntimeBroker,
		enrollResponseTagRosterSnapshot,
	}
	rosterEntryAllowedTags = []uint64{
		rosterEntryTagPeerID,
		rosterEntryTagMemberCredential,
		rosterEntryTagDeviceName,
		rosterEntryTagPlatform,
	}
)

// MarshalBinary encodes the invite capability as canonical TLV.
func (i InviteCapability) MarshalBinary() ([]byte, error) {
	normalized, err := normalizeInviteCapability(i)
	if err != nil {
		return nil, err
	}
	return marshalInviteCapability(normalized, true)
}

// InviteCode returns the MPINV1 textual representation.
func (i InviteCapability) InviteCode() (string, error) {
	data, err := i.MarshalBinary()
	if err != nil {
		return "", err
	}
	return MPINV1Prefix + base64.RawURLEncoding.EncodeToString(data), nil
}

// UnmarshalInviteCapability decodes one signed invite capability from TLV.
func UnmarshalInviteCapability(data []byte) (InviteCapability, error) {
	fields, err := wire.DecodeFieldsStrict(data, inviteAllowedTags...)
	if err != nil {
		return InviteCapability{}, err
	}
	index := indexFields(fields)

	brokerEndpoint, err := requireASCIIField(index, inviteTagBrokerEndpoint, "broker")
	if err != nil {
		return InviteCapability{}, err
	}
	joinTopic, err := requireASCIIField(index, inviteTagJoinTopic, "join_topic")
	if err != nil {
		return InviteCapability{}, err
	}
	inviteID, err := requireASCIIField(index, inviteTagInviteID, "invite_id")
	if err != nil {
		return InviteCapability{}, err
	}
	notAfterUnixMs, err := requireU64Field(index, inviteTagNotAfterUnixMs, "not_after")
	if err != nil {
		return InviteCapability{}, err
	}

	networkIDBytes, err := requireBytesField(index, inviteTagNetworkIDBytes, "network_id_bytes")
	if err != nil {
		return InviteCapability{}, err
	}
	authorityEd25519Pub, err := requireBytesField(index, inviteTagAuthorityEd25519Pub, "authority_ed25519_pub")
	if err != nil {
		return InviteCapability{}, err
	}
	authorityX25519Pub, err := requireBytesField(index, inviteTagAuthorityX25519Pub, "authority_x25519_pub")
	if err != nil {
		return InviteCapability{}, err
	}
	signature, err := requireBytesField(index, inviteTagSignature, "signature")
	if err != nil {
		return InviteCapability{}, err
	}

	return normalizeInviteCapability(InviteCapability{
		NetworkIDBytes:      networkIDBytes,
		AuthorityEd25519Pub: authorityEd25519Pub,
		AuthorityX25519Pub:  authorityX25519Pub,
		BrokerEndpoint:      brokerEndpoint,
		JoinTopic:           joinTopic,
		InviteID:            inviteID,
		NotAfterUnixMs:      notAfterUnixMs,
		Signature:           signature,
	})
}

// ParseInviteCode decodes one MPINV1 textual invite code.
func ParseInviteCode(value string) (InviteCapability, error) {
	trimmed := strings.TrimSpace(value)
	if !strings.HasPrefix(trimmed, MPINV1Prefix) {
		return InviteCapability{}, fmt.Errorf("%w: missing %s prefix", ErrInvalidInviteCode, MPINV1Prefix)
	}
	payload, err := base64.RawURLEncoding.DecodeString(strings.TrimPrefix(trimmed, MPINV1Prefix))
	if err != nil {
		return InviteCapability{}, fmt.Errorf("%w: decode payload: %v", ErrInvalidInviteCode, err)
	}
	invite, err := UnmarshalInviteCapability(payload)
	if err != nil {
		return InviteCapability{}, fmt.Errorf("%w: %v", ErrInvalidInviteCode, err)
	}
	return invite, nil
}

// SignInviteCapability signs the invite capability with the authority key.
func SignInviteCapability(priv ed25519.PrivateKey, invite *InviteCapability) error {
	if invite == nil {
		return fmt.Errorf("%w: nil invite capability", ErrInvalidInviteCode)
	}
	if len(priv) != ed25519.PrivateKeySize {
		return fmt.Errorf("%w: invalid authority private key length: %d", ErrInvalidInviteCode, len(priv))
	}
	pub, ok := priv.Public().(ed25519.PublicKey)
	if !ok {
		return fmt.Errorf("%w: authority public key unavailable", ErrInvalidInviteCode)
	}
	if len(invite.AuthorityEd25519Pub) == 0 {
		invite.AuthorityEd25519Pub = append([]byte(nil), pub...)
	}
	if !bytes.Equal(invite.AuthorityEd25519Pub, pub) {
		return fmt.Errorf("%w: authority_ed25519_pub does not match signer", ErrInvalidInviteCode)
	}

	transcript, err := buildInviteTranscript(*invite)
	if err != nil {
		return err
	}
	invite.Signature = ed25519.Sign(priv, transcript)
	return nil
}

// VerifyInviteCapability verifies the signed invite capability.
func VerifyInviteCapability(invite InviteCapability) error {
	normalized, err := normalizeInviteCapability(invite)
	if err != nil {
		return err
	}
	if len(normalized.Signature) != ed25519.SignatureSize {
		return fmt.Errorf("%w: invalid signature length: %d", ErrInvalidInviteCode, len(normalized.Signature))
	}
	transcript, err := buildInviteTranscript(normalized)
	if err != nil {
		return err
	}
	if !ed25519.Verify(normalized.AuthorityEd25519Pub, transcript, normalized.Signature) {
		return wire.ErrInvalidSignature
	}
	return nil
}

// MarshalBinary encodes the join request as canonical TLV.
func (j JoinRequest) MarshalBinary() ([]byte, error) {
	normalized, err := normalizeJoinRequest(j, true)
	if err != nil {
		return nil, err
	}
	return marshalJoinRequest(normalized, true)
}

// UnmarshalJoinRequest decodes one signed join request from TLV.
func UnmarshalJoinRequest(data []byte) (JoinRequest, error) {
	fields, err := wire.DecodeFieldsStrict(data, joinRequestAllowedTags...)
	if err != nil {
		return JoinRequest{}, err
	}
	index := indexFields(fields)

	inviteID, err := requireASCIIField(index, joinRequestTagInviteID, "invite_id")
	if err != nil {
		return JoinRequest{}, err
	}
	replyTopic, err := requireASCIIField(index, joinRequestTagReplyTopic, "reply_topic")
	if err != nil {
		return JoinRequest{}, err
	}
	createdAtUnixMs, err := requireU64Field(index, joinRequestTagCreatedAtUnixMs, "created_at")
	if err != nil {
		return JoinRequest{}, err
	}
	expiresAtUnixMs, err := requireU64Field(index, joinRequestTagExpiresAtUnixMs, "expires_at")
	if err != nil {
		return JoinRequest{}, err
	}

	deviceName, err := optionalASCIIField(index, joinRequestTagDeviceName)
	if err != nil {
		return JoinRequest{}, err
	}
	platform, err := optionalASCIIField(index, joinRequestTagPlatform)
	if err != nil {
		return JoinRequest{}, err
	}

	requesterEd25519Pub, err := requireBytesField(index, joinRequestTagRequesterEd25519Pub, "requester_ed25519_pub")
	if err != nil {
		return JoinRequest{}, err
	}
	requesterX25519Pub, err := requireBytesField(index, joinRequestTagRequesterX25519Pub, "requester_x25519_pub")
	if err != nil {
		return JoinRequest{}, err
	}
	signature, err := requireBytesField(index, joinRequestTagSignature, "signature")
	if err != nil {
		return JoinRequest{}, err
	}

	return normalizeJoinRequest(JoinRequest{
		InviteID:             inviteID,
		RequesterEd25519Pub:  requesterEd25519Pub,
		RequesterX25519Pub:   requesterX25519Pub,
		ReplyTopic:           replyTopic,
		DeviceName:           deviceName,
		Platform:             platform,
		CreatedAtUnixMs:      createdAtUnixMs,
		ExpiresAtUnixMs:      expiresAtUnixMs,
		ProofOfPossessionSig: signature,
	}, true)
}

// SignJoinRequest signs the join request proof-of-possession.
func SignJoinRequest(priv ed25519.PrivateKey, req *JoinRequest) error {
	if req == nil {
		return fmt.Errorf("%w: nil join request", ErrInvalidJoinRequest)
	}
	if len(priv) != ed25519.PrivateKeySize {
		return fmt.Errorf("%w: invalid requester private key length: %d", ErrInvalidJoinRequest, len(priv))
	}
	pub, ok := priv.Public().(ed25519.PublicKey)
	if !ok {
		return fmt.Errorf("%w: requester public key unavailable", ErrInvalidJoinRequest)
	}
	if len(req.RequesterEd25519Pub) == 0 {
		req.RequesterEd25519Pub = append([]byte(nil), pub...)
	}
	if !bytes.Equal(req.RequesterEd25519Pub, pub) {
		return fmt.Errorf("%w: requester_ed25519_pub does not match signer", ErrInvalidJoinRequest)
	}

	transcript, err := buildJoinRequestTranscript(*req)
	if err != nil {
		return err
	}
	req.ProofOfPossessionSig = ed25519.Sign(priv, transcript)
	return nil
}

// VerifyJoinRequest verifies the join request proof-of-possession.
func VerifyJoinRequest(req JoinRequest) error {
	normalized, err := normalizeJoinRequest(req, true)
	if err != nil {
		return err
	}
	transcript, err := buildJoinRequestTranscript(normalized)
	if err != nil {
		return err
	}
	if !ed25519.Verify(normalized.RequesterEd25519Pub, transcript, normalized.ProofOfPossessionSig) {
		return wire.ErrInvalidSignature
	}
	return nil
}

// MarshalBinary encodes the member credential as canonical TLV.
func (m MemberCredential) MarshalBinary() ([]byte, error) {
	normalized, err := normalizeMemberCredential(m, true)
	if err != nil {
		return nil, err
	}
	return marshalMemberCredential(normalized, true)
}

// UnmarshalMemberCredential decodes one signed member credential from TLV.
func UnmarshalMemberCredential(data []byte) (MemberCredential, error) {
	fields, err := wire.DecodeFieldsStrict(data, memberCredentialAllowedTags...)
	if err != nil {
		return MemberCredential{}, err
	}
	index := indexFields(fields)

	networkID, err := requireASCIIField(index, memberCredentialTagNetworkID, "network_id")
	if err != nil {
		return MemberCredential{}, err
	}
	role, err := requireASCIIField(index, memberCredentialTagRole, "role")
	if err != nil {
		return MemberCredential{}, err
	}
	notBeforeUnixMs, err := requireU64Field(index, memberCredentialTagNotBeforeUnixMs, "not_before")
	if err != nil {
		return MemberCredential{}, err
	}
	notAfterUnixMs, err := requireU64Field(index, memberCredentialTagNotAfterUnixMs, "not_after")
	if err != nil {
		return MemberCredential{}, err
	}
	issuerKeyID, err := requireASCIIField(index, memberCredentialTagIssuerKeyID, "issuer_key_id")
	if err != nil {
		return MemberCredential{}, err
	}

	subjectEd25519Pub, err := requireBytesField(index, memberCredentialTagSubjectEd25519, "subject_ed25519_pub")
	if err != nil {
		return MemberCredential{}, err
	}
	subjectX25519Pub, err := requireBytesField(index, memberCredentialTagSubjectX25519, "subject_x25519_pub")
	if err != nil {
		return MemberCredential{}, err
	}
	signature, err := requireBytesField(index, memberCredentialTagSignature, "signature")
	if err != nil {
		return MemberCredential{}, err
	}

	return normalizeMemberCredential(MemberCredential{
		NetworkID:         networkID,
		SubjectEd25519Pub: subjectEd25519Pub,
		SubjectX25519Pub:  subjectX25519Pub,
		Role:              role,
		NotBeforeUnixMs:   notBeforeUnixMs,
		NotAfterUnixMs:    notAfterUnixMs,
		IssuerKeyID:       issuerKeyID,
		Signature:         signature,
	}, true)
}

// SignMemberCredential signs the member credential with the authority key.
func SignMemberCredential(priv ed25519.PrivateKey, credential *MemberCredential) error {
	if credential == nil {
		return fmt.Errorf("%w: nil member credential", ErrInvalidMemberCredential)
	}
	if len(priv) != ed25519.PrivateKeySize {
		return fmt.Errorf("%w: invalid issuer private key length: %d", ErrInvalidMemberCredential, len(priv))
	}
	transcript, err := buildMemberCredentialTranscript(*credential)
	if err != nil {
		return err
	}
	credential.Signature = ed25519.Sign(priv, transcript)
	return nil
}

// VerifyMemberCredential verifies the credential against one issuer key.
func VerifyMemberCredential(credential MemberCredential, issuer ed25519.PublicKey) error {
	normalized, err := normalizeMemberCredential(credential, true)
	if err != nil {
		return err
	}
	if len(issuer) != ed25519.PublicKeySize {
		return fmt.Errorf("%w: invalid issuer public key length: %d", ErrInvalidMemberCredential, len(issuer))
	}
	transcript, err := buildMemberCredentialTranscript(normalized)
	if err != nil {
		return err
	}
	if !ed25519.Verify(issuer, transcript, normalized.Signature) {
		return wire.ErrInvalidSignature
	}
	return nil
}

// MarshalBinary encodes the enroll response body as canonical TLV.
func (r EnrollResponse) MarshalBinary() ([]byte, error) {
	normalized, err := normalizeEnrollResponse(r)
	if err != nil {
		return nil, err
	}
	return marshalEnrollResponse(normalized)
}

// UnmarshalEnrollResponse decodes one enroll response body from TLV.
func UnmarshalEnrollResponse(data []byte) (EnrollResponse, error) {
	fields, err := wire.DecodeFieldsStrict(data, enrollResponseAllowedTags...)
	if err != nil {
		return EnrollResponse{}, err
	}
	index := indexFields(fields)

	selfCredentialBytes, err := requireBytesField(index, enrollResponseTagSelfMemberCredential, "self_member_credential")
	if err != nil {
		return EnrollResponse{}, err
	}
	memberCredential, err := UnmarshalMemberCredential(selfCredentialBytes)
	if err != nil {
		return EnrollResponse{}, err
	}
	brokerEndpoint, err := requireASCIIField(index, enrollResponseTagRuntimeBroker, "runtime_broker")
	if err != nil {
		return EnrollResponse{}, err
	}
	mailboxSecret, err := requireBytesField(index, enrollResponseTagMailboxSecret, "mailbox_secret")
	if err != nil {
		return EnrollResponse{}, err
	}
	rosterBytes, err := requireBytesField(index, enrollResponseTagRosterSnapshot, "roster_snapshot")
	if err != nil {
		return EnrollResponse{}, err
	}
	rosterSnapshot, err := unmarshalRosterSnapshot(rosterBytes)
	if err != nil {
		return EnrollResponse{}, err
	}

	return normalizeEnrollResponse(EnrollResponse{
		SelfMemberCredential: memberCredential,
		MailboxSecret:        mailboxSecret,
		RuntimeBroker:        RuntimeBroker{Endpoint: brokerEndpoint},
		RosterSnapshot:       rosterSnapshot,
	})
}

func normalizeEnrollResponse(r EnrollResponse) (EnrollResponse, error) {
	credential, err := normalizeMemberCredential(r.SelfMemberCredential, true)
	if err != nil {
		return EnrollResponse{}, err
	}
	if len(r.MailboxSecret) != mailboxSecretSize {
		return EnrollResponse{}, fmt.Errorf("%w: invalid mailbox secret length: %d", ErrInvalidJoinRequest, len(r.MailboxSecret))
	}
	broker, err := normalizeRuntimeBroker(r.RuntimeBroker)
	if err != nil {
		return EnrollResponse{}, err
	}
	snapshot, err := normalizeRosterSnapshot(r.RosterSnapshot)
	if err != nil {
		return EnrollResponse{}, err
	}
	return EnrollResponse{
		SelfMemberCredential: credential,
		MailboxSecret:        append([]byte(nil), r.MailboxSecret...),
		RuntimeBroker:        broker,
		RosterSnapshot:       snapshot,
	}, nil
}

func marshalInviteCapability(invite InviteCapability, includeSignature bool) ([]byte, error) {
	out := make([]byte, 0, 192)
	var err error
	out = wire.AppendBytesField(out, inviteTagNetworkIDBytes, invite.NetworkIDBytes)
	out = wire.AppendBytesField(out, inviteTagAuthorityEd25519Pub, invite.AuthorityEd25519Pub)
	out = wire.AppendBytesField(out, inviteTagAuthorityX25519Pub, invite.AuthorityX25519Pub)
	out, err = wire.AppendASCIIField(out, inviteTagBrokerEndpoint, invite.BrokerEndpoint)
	if err != nil {
		return nil, err
	}
	out, err = wire.AppendASCIIField(out, inviteTagJoinTopic, invite.JoinTopic)
	if err != nil {
		return nil, err
	}
	out, err = wire.AppendASCIIField(out, inviteTagInviteID, invite.InviteID)
	if err != nil {
		return nil, err
	}
	out = wire.AppendU64Field(out, inviteTagNotAfterUnixMs, invite.NotAfterUnixMs)
	if includeSignature {
		out = wire.AppendBytesField(out, inviteTagSignature, invite.Signature)
	}
	return out, nil
}

func marshalJoinRequest(req JoinRequest, includeSignature bool) ([]byte, error) {
	out := make([]byte, 0, 192)
	var err error
	out, err = wire.AppendASCIIField(out, joinRequestTagInviteID, req.InviteID)
	if err != nil {
		return nil, err
	}
	out = wire.AppendBytesField(out, joinRequestTagRequesterEd25519Pub, req.RequesterEd25519Pub)
	out = wire.AppendBytesField(out, joinRequestTagRequesterX25519Pub, req.RequesterX25519Pub)
	out, err = wire.AppendASCIIField(out, joinRequestTagReplyTopic, req.ReplyTopic)
	if err != nil {
		return nil, err
	}
	if req.DeviceName != "" {
		out, err = wire.AppendASCIIField(out, joinRequestTagDeviceName, req.DeviceName)
		if err != nil {
			return nil, err
		}
	}
	if req.Platform != "" {
		out, err = wire.AppendASCIIField(out, joinRequestTagPlatform, req.Platform)
		if err != nil {
			return nil, err
		}
	}
	out = wire.AppendU64Field(out, joinRequestTagCreatedAtUnixMs, req.CreatedAtUnixMs)
	out = wire.AppendU64Field(out, joinRequestTagExpiresAtUnixMs, req.ExpiresAtUnixMs)
	if includeSignature {
		out = wire.AppendBytesField(out, joinRequestTagSignature, req.ProofOfPossessionSig)
	}
	return out, nil
}

func marshalMemberCredential(credential MemberCredential, includeSignature bool) ([]byte, error) {
	out := make([]byte, 0, 192)
	var err error
	out, err = wire.AppendASCIIField(out, memberCredentialTagNetworkID, credential.NetworkID)
	if err != nil {
		return nil, err
	}
	out = wire.AppendBytesField(out, memberCredentialTagSubjectEd25519, credential.SubjectEd25519Pub)
	out = wire.AppendBytesField(out, memberCredentialTagSubjectX25519, credential.SubjectX25519Pub)
	out, err = wire.AppendASCIIField(out, memberCredentialTagRole, credential.Role)
	if err != nil {
		return nil, err
	}
	out = wire.AppendU64Field(out, memberCredentialTagNotBeforeUnixMs, credential.NotBeforeUnixMs)
	out = wire.AppendU64Field(out, memberCredentialTagNotAfterUnixMs, credential.NotAfterUnixMs)
	out, err = wire.AppendASCIIField(out, memberCredentialTagIssuerKeyID, credential.IssuerKeyID)
	if err != nil {
		return nil, err
	}
	if includeSignature {
		out = wire.AppendBytesField(out, memberCredentialTagSignature, credential.Signature)
	}
	return out, nil
}

func marshalEnrollResponse(response EnrollResponse) ([]byte, error) {
	credential, err := response.SelfMemberCredential.MarshalBinary()
	if err != nil {
		return nil, err
	}
	rosterBytes, err := marshalRosterSnapshot(response.RosterSnapshot)
	if err != nil {
		return nil, err
	}

	out := make([]byte, 0, len(credential)+len(rosterBytes)+128)
	out = wire.AppendBytesField(out, enrollResponseTagSelfMemberCredential, credential)
	out = wire.AppendBytesField(out, enrollResponseTagMailboxSecret, response.MailboxSecret)
	out, err = wire.AppendASCIIField(out, enrollResponseTagRuntimeBroker, response.RuntimeBroker.Endpoint)
	if err != nil {
		return nil, err
	}
	out = wire.AppendBytesField(out, enrollResponseTagRosterSnapshot, rosterBytes)
	return out, nil
}

func marshalRosterSnapshot(snapshot RosterSnapshot) ([]byte, error) {
	normalized, err := normalizeRosterSnapshot(snapshot)
	if err != nil {
		return nil, err
	}
	out := make([]byte, 0, len(normalized.Entries)*128)
	for _, entry := range normalized.Entries {
		entryBytes, err := marshalRosterEntry(entry)
		if err != nil {
			return nil, err
		}
		out = appendLengthPrefixed(out, entryBytes)
	}
	return out, nil
}

func marshalRosterEntry(entry RosterEntry) ([]byte, error) {
	credential, err := entry.MemberCredential.MarshalBinary()
	if err != nil {
		return nil, err
	}

	out := make([]byte, 0, len(credential)+96)
	out, err = wire.AppendASCIIField(out, rosterEntryTagPeerID, entry.PeerID)
	if err != nil {
		return nil, err
	}
	out = wire.AppendBytesField(out, rosterEntryTagMemberCredential, credential)
	if entry.DeviceName != "" {
		out, err = wire.AppendASCIIField(out, rosterEntryTagDeviceName, entry.DeviceName)
		if err != nil {
			return nil, err
		}
	}
	if entry.Platform != "" {
		out, err = wire.AppendASCIIField(out, rosterEntryTagPlatform, entry.Platform)
		if err != nil {
			return nil, err
		}
	}
	return out, nil
}

func unmarshalRosterSnapshot(data []byte) (RosterSnapshot, error) {
	entriesData, err := decodeLengthPrefixedList(data)
	if err != nil {
		return RosterSnapshot{}, err
	}
	entries := make([]RosterEntry, 0, len(entriesData))
	for i, entryData := range entriesData {
		entry, err := unmarshalRosterEntry(entryData)
		if err != nil {
			return RosterSnapshot{}, fmt.Errorf("roster entry %d: %w", i, err)
		}
		entries = append(entries, entry)
	}
	return normalizeRosterSnapshot(RosterSnapshot{Entries: entries})
}

func unmarshalRosterEntry(data []byte) (RosterEntry, error) {
	fields, err := wire.DecodeFieldsStrict(data, rosterEntryAllowedTags...)
	if err != nil {
		return RosterEntry{}, err
	}
	index := indexFields(fields)
	peerID, err := requireASCIIField(index, rosterEntryTagPeerID, "peer_id")
	if err != nil {
		return RosterEntry{}, err
	}
	credentialBytes, err := requireBytesField(index, rosterEntryTagMemberCredential, "member_credential")
	if err != nil {
		return RosterEntry{}, err
	}
	credential, err := UnmarshalMemberCredential(credentialBytes)
	if err != nil {
		return RosterEntry{}, err
	}
	deviceName, err := optionalASCIIField(index, rosterEntryTagDeviceName)
	if err != nil {
		return RosterEntry{}, err
	}
	platform, err := optionalASCIIField(index, rosterEntryTagPlatform)
	if err != nil {
		return RosterEntry{}, err
	}
	entry := RosterEntry{
		PeerID:           peerID,
		MemberCredential: credential,
		DeviceName:       deviceName,
		Platform:         platform,
	}
	if err := normalizeRosterEntry(entry); err != nil {
		return RosterEntry{}, err
	}
	return entry, nil
}

func buildInviteTranscript(invite InviteCapability) ([]byte, error) {
	normalized, err := normalizeInviteCapability(invite)
	if err != nil {
		return nil, err
	}
	body, err := marshalInviteCapability(normalized, false)
	if err != nil {
		return nil, err
	}
	return buildTranscript(inviteTranscriptDomain, body), nil
}

func buildJoinRequestTranscript(req JoinRequest) ([]byte, error) {
	normalized, err := normalizeJoinRequest(req, false)
	if err != nil {
		return nil, err
	}
	body, err := marshalJoinRequest(normalized, false)
	if err != nil {
		return nil, err
	}
	return buildTranscript(joinRequestTranscriptDomain, body), nil
}

func buildMemberCredentialTranscript(credential MemberCredential) ([]byte, error) {
	normalized, err := normalizeMemberCredential(credential, false)
	if err != nil {
		return nil, err
	}
	body, err := marshalMemberCredential(normalized, false)
	if err != nil {
		return nil, err
	}
	return buildTranscript(memberCredentialTranscriptDomain, body), nil
}

func buildTranscript(domain string, body []byte) []byte {
	out := make([]byte, 0, len(domain)+len(body)+1)
	out = append(out, domain...)
	out = append(out, 0x00)
	out = append(out, body...)
	return out
}

func indexFields(fields []wire.DecodedField) map[uint64]wire.DecodedField {
	index := make(map[uint64]wire.DecodedField, len(fields))
	for _, field := range fields {
		index[field.Tag] = field
	}
	return index
}

func requireASCIIField(index map[uint64]wire.DecodedField, tag uint64, name string) (string, error) {
	field, ok := index[tag]
	if !ok {
		return "", fmt.Errorf("%w: missing %s", wire.ErrInvalidFieldValue, name)
	}
	return wire.DecodeASCIIField(field)
}

func optionalASCIIField(index map[uint64]wire.DecodedField, tag uint64) (string, error) {
	field, ok := index[tag]
	if !ok {
		return "", nil
	}
	return wire.DecodeASCIIField(field)
}

func requireU64Field(index map[uint64]wire.DecodedField, tag uint64, name string) (uint64, error) {
	field, ok := index[tag]
	if !ok {
		return 0, fmt.Errorf("%w: missing %s", wire.ErrInvalidFieldValue, name)
	}
	return wire.DecodeU64Field(field)
}

func requireBytesField(index map[uint64]wire.DecodedField, tag uint64, name string) ([]byte, error) {
	field, ok := index[tag]
	if !ok {
		return nil, fmt.Errorf("%w: missing %s", wire.ErrInvalidFieldValue, name)
	}
	return append([]byte(nil), field.Value...), nil
}

func appendLengthPrefixed(dst []byte, value []byte) []byte {
	var buf [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(buf[:], uint64(len(value)))
	dst = append(dst, buf[:n]...)
	return append(dst, value...)
}

func decodeLengthPrefixedList(data []byte) ([][]byte, error) {
	out := make([][]byte, 0, 4)
	offset := 0
	for offset < len(data) {
		length, n, err := decodeCanonicalUvarint(data[offset:])
		if err != nil {
			return nil, err
		}
		offset += n
		if length > uint64(len(data)-offset) {
			return nil, fmt.Errorf("%w: roster snapshot entry length %d exceeds remaining %d", wire.ErrTruncatedTLV, length, len(data)-offset)
		}
		out = append(out, append([]byte(nil), data[offset:offset+int(length)]...))
		offset += int(length)
	}
	return out, nil
}

func decodeCanonicalUvarint(data []byte) (uint64, int, error) {
	value, n := binary.Uvarint(data)
	switch {
	case n == 0:
		return 0, 0, fmt.Errorf("%w: truncated uvarint", wire.ErrTruncatedTLV)
	case n < 0:
		return 0, 0, fmt.Errorf("%w: overflow", wire.ErrNonCanonicalUvarint)
	}
	var buf [binary.MaxVarintLen64]byte
	m := binary.PutUvarint(buf[:], value)
	if m != n || !bytes.Equal(buf[:m], data[:n]) {
		return 0, 0, fmt.Errorf("%w", wire.ErrNonCanonicalUvarint)
	}
	return value, n, nil
}
