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
	"crypto/ed25519"
	"fmt"
	"strings"

	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

const mailboxSecretSize = 32

// InviteCapability is the current v1 entry ticket for enrollment.
type InviteCapability struct {
	NetworkIDBytes      []byte
	AuthorityEd25519Pub []byte
	AuthorityX25519Pub  []byte
	BrokerEndpoint      string
	JoinTopic           string
	InviteID            string
	NotAfterUnixMs      uint64
	Signature           []byte
}

// NetworkID returns the canonical network identifier derived from the invite.
func (i InviteCapability) NetworkID() (string, error) {
	return wire.EncodeNetworkID(i.NetworkIDBytes)
}

// JoinRequest is the current v1 enrollment request body.
type JoinRequest struct {
	InviteID             string
	RequesterEd25519Pub  []byte
	RequesterX25519Pub   []byte
	ReplyTopic           string
	DeviceName           string
	Platform             string
	CreatedAtUnixMs      uint64
	ExpiresAtUnixMs      uint64
	ProofOfPossessionSig []byte
}

// PeerID derives the canonical peer_id from the requester's signing key.
func (j JoinRequest) PeerID() (string, error) {
	if len(j.RequesterEd25519Pub) != ed25519.PublicKeySize {
		return "", fmt.Errorf("%w: invalid ed25519 public key length: %d", ErrInvalidJoinRequest, len(j.RequesterEd25519Pub))
	}
	return wire.PeerIDFromEd25519Pub(j.RequesterEd25519Pub)
}

// MemberCredential binds a subject's keys to one network.
type MemberCredential struct {
	NetworkID         string
	SubjectEd25519Pub []byte
	SubjectX25519Pub  []byte
	Role              string
	NotBeforeUnixMs   uint64
	NotAfterUnixMs    uint64
	IssuerKeyID       string
	Signature         []byte
}

// PeerID derives the canonical peer_id from the subject signing key.
func (m MemberCredential) PeerID() (string, error) {
	if len(m.SubjectEd25519Pub) != ed25519.PublicKeySize {
		return "", fmt.Errorf("%w: invalid ed25519 public key length: %d", ErrInvalidMemberCredential, len(m.SubjectEd25519Pub))
	}
	return wire.PeerIDFromEd25519Pub(m.SubjectEd25519Pub)
}

// RosterEntry is one trusted member roster entry.
type RosterEntry struct {
	PeerID           string
	MemberCredential MemberCredential
	DeviceName       string
	Platform         string
}

// RosterSnapshot is the initial trusted roster bundled with enrollment.
type RosterSnapshot struct {
	Entries []RosterEntry
}

// RuntimeBroker carries the single runtime broker endpoint and optional
// ordinary STUN servers for runtime candidate gathering.
type RuntimeBroker struct {
	Endpoint    string
	StunServers []string
}

// EnrollResponse is the sealed bootstrap package delivered after approval.
type EnrollResponse struct {
	SelfMemberCredential MemberCredential
	MailboxSecret        []byte
	RuntimeBroker        RuntimeBroker
	RosterSnapshot       RosterSnapshot
}

// RequestFingerprint captures the stable join-request identity for replay.
type RequestFingerprint struct {
	InviteID            string
	RequesterEd25519Pub []byte
	RequesterX25519Pub  []byte
	ReplyTopic          string
	DeviceName          string
	Platform            string
	CreatedAtUnixMs     uint64
	ExpiresAtUnixMs     uint64
	BodySHA256          [32]byte
}

// HandledJoinRequest stores the cached response ciphertext for one msg_id.
type HandledJoinRequest struct {
	MsgID              string
	RequestFingerprint RequestFingerprint
	ResponseCiphertext []byte
}

// JoinedBootstrap converts the enroll response into persistence handoff.
func (r EnrollResponse) JoinedBootstrap() (persist.JoinedBootstrap, error) {
	normalized, err := normalizeEnrollResponse(r)
	if err != nil {
		return persist.JoinedBootstrap{}, err
	}
	credential, err := normalized.SelfMemberCredential.MarshalBinary()
	if err != nil {
		return persist.JoinedBootstrap{}, err
	}
	roster, err := normalized.RosterSnapshot.ToPersist()
	if err != nil {
		return persist.JoinedBootstrap{}, err
	}

	return persist.JoinedBootstrap{
		NetworkID:            normalized.SelfMemberCredential.NetworkID,
		SelfMemberCredential: credential,
		MailboxSecret:        append([]byte(nil), normalized.MailboxSecret...),
		RuntimeBroker: persist.RuntimeBroker{
			Endpoint:    normalized.RuntimeBroker.Endpoint,
			StunServers: normalized.RuntimeBroker.StunServers,
		},
		RosterSnapshot: roster,
	}, nil
}

// ToPersist converts one roster entry into the persistence model.
func (e RosterEntry) ToPersist() (persist.RosterEntry, error) {
	if err := normalizeRosterEntry(e); err != nil {
		return persist.RosterEntry{}, err
	}
	credential, err := e.MemberCredential.MarshalBinary()
	if err != nil {
		return persist.RosterEntry{}, err
	}
	return persist.RosterEntry{
		PeerID:           strings.TrimSpace(e.PeerID),
		MemberCredential: credential,
		DeviceName:       strings.TrimSpace(e.DeviceName),
		Platform:         strings.TrimSpace(e.Platform),
	}, nil
}

// ToPersist converts the roster snapshot into the persistence model.
func (r RosterSnapshot) ToPersist() (persist.RosterSnapshot, error) {
	normalized, err := normalizeRosterSnapshot(r)
	if err != nil {
		return persist.RosterSnapshot{}, err
	}
	entries := make([]persist.RosterEntry, 0, len(normalized.Entries))
	for i, entry := range normalized.Entries {
		persistEntry, err := entry.ToPersist()
		if err != nil {
			return persist.RosterSnapshot{}, fmt.Errorf("roster entry %d: %w", i, err)
		}
		entries = append(entries, persistEntry)
	}
	return persist.RosterSnapshot{Entries: entries}, nil
}

func normalizeRuntimeBroker(b RuntimeBroker) (RuntimeBroker, error) {
	endpoint := strings.TrimSpace(b.Endpoint)
	if endpoint == "" {
		return RuntimeBroker{}, fmt.Errorf("%w: empty runtime broker endpoint", ErrInvalidBrokerEndpoint)
	}
	return RuntimeBroker{
		Endpoint:    endpoint,
		StunServers: normalizeStunServers(b.StunServers),
	}, nil
}

func normalizeStunServers(servers []string) []string {
	normalized := make([]string, 0, len(servers))
	seen := make(map[string]struct{}, len(servers))
	for _, server := range servers {
		trimmed := strings.TrimSpace(server)
		if trimmed == "" {
			continue
		}
		if _, ok := seen[trimmed]; ok {
			continue
		}
		seen[trimmed] = struct{}{}
		normalized = append(normalized, trimmed)
	}
	return normalized
}

func normalizeInviteCapability(i InviteCapability) (InviteCapability, error) {
	networkIDBytes := append([]byte(nil), i.NetworkIDBytes...)
	if len(networkIDBytes) != wire.RawIDLen {
		return InviteCapability{}, fmt.Errorf("%w: invalid network_id_bytes length: %d", ErrInvalidInviteCode, len(networkIDBytes))
	}
	if len(i.AuthorityEd25519Pub) != ed25519.PublicKeySize {
		return InviteCapability{}, fmt.Errorf("%w: invalid authority ed25519 public key length: %d", ErrInvalidInviteCode, len(i.AuthorityEd25519Pub))
	}
	if len(i.AuthorityX25519Pub) != 32 {
		return InviteCapability{}, fmt.Errorf("%w: invalid authority x25519 public key length: %d", ErrInvalidInviteCode, len(i.AuthorityX25519Pub))
	}
	broker, err := normalizeRuntimeBroker(RuntimeBroker{Endpoint: i.BrokerEndpoint})
	if err != nil {
		return InviteCapability{}, err
	}
	joinTopic := strings.TrimSpace(i.JoinTopic)
	if joinTopic == "" {
		return InviteCapability{}, fmt.Errorf("%w: empty join_topic", ErrInvalidInviteCode)
	}
	inviteID, err := wire.CanonicalizeMsgID(i.InviteID)
	if err != nil {
		return InviteCapability{}, fmt.Errorf("%w: canonicalize invite_id: %w", ErrInvalidInviteCode, err)
	}
	if i.NotAfterUnixMs == 0 {
		return InviteCapability{}, fmt.Errorf("%w: missing not_after", ErrInvalidInviteCode)
	}
	return InviteCapability{
		NetworkIDBytes:      networkIDBytes,
		AuthorityEd25519Pub: append([]byte(nil), i.AuthorityEd25519Pub...),
		AuthorityX25519Pub:  append([]byte(nil), i.AuthorityX25519Pub...),
		BrokerEndpoint:      broker.Endpoint,
		JoinTopic:           joinTopic,
		InviteID:            inviteID,
		NotAfterUnixMs:      i.NotAfterUnixMs,
		Signature:           append([]byte(nil), i.Signature...),
	}, nil
}

func normalizeJoinRequest(j JoinRequest, requireSignature bool) (JoinRequest, error) {
	inviteID, err := wire.CanonicalizeMsgID(j.InviteID)
	if err != nil {
		return JoinRequest{}, fmt.Errorf("%w: canonicalize invite_id: %w", ErrInvalidJoinRequest, err)
	}
	if len(j.RequesterEd25519Pub) != ed25519.PublicKeySize {
		return JoinRequest{}, fmt.Errorf("%w: invalid requester ed25519 public key length: %d", ErrInvalidJoinRequest, len(j.RequesterEd25519Pub))
	}
	if len(j.RequesterX25519Pub) != 32 {
		return JoinRequest{}, fmt.Errorf("%w: invalid requester x25519 public key length: %d", ErrInvalidJoinRequest, len(j.RequesterX25519Pub))
	}
	replyTopic := strings.TrimSpace(j.ReplyTopic)
	if replyTopic == "" {
		return JoinRequest{}, fmt.Errorf("%w: empty reply_topic", ErrInvalidJoinRequest)
	}
	if j.CreatedAtUnixMs == 0 {
		return JoinRequest{}, fmt.Errorf("%w: missing created_at", ErrInvalidJoinRequest)
	}
	if j.ExpiresAtUnixMs == 0 {
		return JoinRequest{}, fmt.Errorf("%w: missing expires_at", ErrInvalidJoinRequest)
	}
	if j.CreatedAtUnixMs > j.ExpiresAtUnixMs {
		return JoinRequest{}, fmt.Errorf("%w: created_at > expires_at", ErrInvalidJoinRequest)
	}
	sig := append([]byte(nil), j.ProofOfPossessionSig...)
	if requireSignature && len(sig) != ed25519.SignatureSize {
		return JoinRequest{}, fmt.Errorf("%w: invalid proof-of-possession signature length: %d", ErrInvalidJoinRequest, len(sig))
	}
	if !requireSignature {
		sig = nil
	}

	return JoinRequest{
		InviteID:             inviteID,
		RequesterEd25519Pub:  append([]byte(nil), j.RequesterEd25519Pub...),
		RequesterX25519Pub:   append([]byte(nil), j.RequesterX25519Pub...),
		ReplyTopic:           replyTopic,
		DeviceName:           strings.TrimSpace(j.DeviceName),
		Platform:             strings.TrimSpace(j.Platform),
		CreatedAtUnixMs:      j.CreatedAtUnixMs,
		ExpiresAtUnixMs:      j.ExpiresAtUnixMs,
		ProofOfPossessionSig: sig,
	}, nil
}

func normalizeMemberCredential(m MemberCredential, requireSignature bool) (MemberCredential, error) {
	networkID, err := wire.CanonicalizeNetworkID(m.NetworkID)
	if err != nil {
		return MemberCredential{}, fmt.Errorf("%w: canonicalize network_id: %w", ErrInvalidMemberCredential, err)
	}
	if len(m.SubjectEd25519Pub) != ed25519.PublicKeySize {
		return MemberCredential{}, fmt.Errorf("%w: invalid subject ed25519 public key length: %d", ErrInvalidMemberCredential, len(m.SubjectEd25519Pub))
	}
	if len(m.SubjectX25519Pub) != 32 {
		return MemberCredential{}, fmt.Errorf("%w: invalid subject x25519 public key length: %d", ErrInvalidMemberCredential, len(m.SubjectX25519Pub))
	}
	role := strings.TrimSpace(m.Role)
	if role == "" {
		return MemberCredential{}, fmt.Errorf("%w: empty role", ErrInvalidMemberCredential)
	}
	issuerKeyID := strings.TrimSpace(m.IssuerKeyID)
	if issuerKeyID == "" {
		return MemberCredential{}, fmt.Errorf("%w: empty issuer_key_id", ErrInvalidMemberCredential)
	}
	if m.NotBeforeUnixMs == 0 {
		return MemberCredential{}, fmt.Errorf("%w: missing not_before", ErrInvalidMemberCredential)
	}
	if m.NotAfterUnixMs == 0 {
		return MemberCredential{}, fmt.Errorf("%w: missing not_after", ErrInvalidMemberCredential)
	}
	if m.NotBeforeUnixMs > m.NotAfterUnixMs {
		return MemberCredential{}, fmt.Errorf("%w: not_before > not_after", ErrInvalidMemberCredential)
	}
	sig := append([]byte(nil), m.Signature...)
	if requireSignature && len(sig) != ed25519.SignatureSize {
		return MemberCredential{}, fmt.Errorf("%w: invalid signature length: %d", ErrInvalidMemberCredential, len(sig))
	}
	if !requireSignature {
		sig = nil
	}

	return MemberCredential{
		NetworkID:         networkID,
		SubjectEd25519Pub: append([]byte(nil), m.SubjectEd25519Pub...),
		SubjectX25519Pub:  append([]byte(nil), m.SubjectX25519Pub...),
		Role:              role,
		NotBeforeUnixMs:   m.NotBeforeUnixMs,
		NotAfterUnixMs:    m.NotAfterUnixMs,
		IssuerKeyID:       issuerKeyID,
		Signature:         sig,
	}, nil
}

func normalizeRosterEntry(e RosterEntry) error {
	peerID, err := wire.CanonicalizePeerID(e.PeerID)
	if err != nil {
		return fmt.Errorf("peer_id: %w", err)
	}
	cred, err := normalizeMemberCredential(e.MemberCredential, true)
	if err != nil {
		return fmt.Errorf("member_credential: %w", err)
	}
	derivedPeerID, err := cred.PeerID()
	if err != nil {
		return err
	}
	if peerID != derivedPeerID {
		return fmt.Errorf("%w: roster entry peer_id does not match member_credential", ErrInvalidRosterSnapshot)
	}
	return nil
}

func normalizeRosterSnapshot(snapshot RosterSnapshot) (RosterSnapshot, error) {
	entries := make([]RosterEntry, 0, len(snapshot.Entries))
	seen := make(map[string]struct{}, len(snapshot.Entries))
	for i, entry := range snapshot.Entries {
		peerID, err := wire.CanonicalizePeerID(entry.PeerID)
		if err != nil {
			return RosterSnapshot{}, fmt.Errorf("roster entry %d peer_id: %w", i, err)
		}
		normalizedCred, err := normalizeMemberCredential(entry.MemberCredential, true)
		if err != nil {
			return RosterSnapshot{}, fmt.Errorf("roster entry %d member_credential: %w", i, err)
		}
		derivedPeerID, err := normalizedCred.PeerID()
		if err != nil {
			return RosterSnapshot{}, fmt.Errorf("roster entry %d peer_id derivation: %w", i, err)
		}
		if peerID != derivedPeerID {
			return RosterSnapshot{}, fmt.Errorf("%w: roster entry %d peer_id does not match member_credential", ErrInvalidRosterSnapshot, i)
		}
		if _, ok := seen[peerID]; ok {
			return RosterSnapshot{}, fmt.Errorf("%w: duplicate peer_id %s", ErrInvalidRosterSnapshot, peerID)
		}
		seen[peerID] = struct{}{}
		entries = append(entries, RosterEntry{
			PeerID:           peerID,
			MemberCredential: normalizedCred,
			DeviceName:       strings.TrimSpace(entry.DeviceName),
			Platform:         strings.TrimSpace(entry.Platform),
		})
	}
	return RosterSnapshot{Entries: entries}, nil
}
