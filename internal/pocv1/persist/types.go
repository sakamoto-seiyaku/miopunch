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

package persist

import (
	"crypto/ed25519"
	"fmt"
	"strings"

	"github.com/miopunch/miopunch/internal/pocv1/wire"
	"golang.org/x/crypto/curve25519"
)

const (
	deviceKeySize     = 32
	mailboxSecretSize = 32
)

// DeviceKeys carries the current v1 device-global identity material as raw
// private key bytes.
type DeviceKeys struct {
	Ed25519Seed      []byte
	X25519PrivateKey []byte
}

// Ed25519PrivateKey derives the Ed25519 private key from the persisted seed.
func (k DeviceKeys) Ed25519PrivateKey() (ed25519.PrivateKey, error) {
	if err := validateDeviceKeys(k); err != nil {
		return nil, err
	}

	seed := append([]byte(nil), k.Ed25519Seed...)
	return ed25519.NewKeyFromSeed(seed), nil
}

// Ed25519PublicKey derives the Ed25519 public key from the persisted seed.
func (k DeviceKeys) Ed25519PublicKey() (ed25519.PublicKey, error) {
	priv, err := k.Ed25519PrivateKey()
	if err != nil {
		return nil, err
	}

	pub := priv.Public().(ed25519.PublicKey)
	return append(ed25519.PublicKey(nil), pub...), nil
}

// X25519PublicKey derives the X25519 public key from the persisted private key.
func (k DeviceKeys) X25519PublicKey() ([]byte, error) {
	if err := validateDeviceKeys(k); err != nil {
		return nil, err
	}

	priv := append([]byte(nil), k.X25519PrivateKey...)
	pub, err := curve25519.X25519(priv, curve25519.Basepoint)
	if err != nil {
		return nil, fmt.Errorf("derive x25519 public key: %w", err)
	}
	return pub, nil
}

// PeerID derives the canonical peer_id from the persisted Ed25519 seed.
func (k DeviceKeys) PeerID() (string, error) {
	pub, err := k.Ed25519PublicKey()
	if err != nil {
		return "", err
	}
	return wire.PeerIDFromEd25519Pub(pub)
}

// RuntimeBroker describes the current v1 single runtime broker endpoint.
type RuntimeBroker struct {
	Endpoint string
}

// RosterEntry is one trusted current v1 member roster entry.
type RosterEntry struct {
	PeerID           string
	MemberCredential []byte
	DeviceName       string
	Platform         string
}

// RosterSnapshot is the whole trusted current v1 roster payload.
type RosterSnapshot struct {
	Entries []RosterEntry
}

// JoinedBootstrap is the grouped joined-network state handoff owned by
// persistence.
type JoinedBootstrap struct {
	NetworkID            string
	SelfMemberCredential []byte
	MailboxSecret        []byte
	RuntimeBroker        RuntimeBroker
	RosterSnapshot       RosterSnapshot
}

func validateDeviceKeys(keys DeviceKeys) error {
	if len(keys.Ed25519Seed) != deviceKeySize {
		return fmt.Errorf("invalid ed25519 seed length: %d", len(keys.Ed25519Seed))
	}
	if len(keys.X25519PrivateKey) != deviceKeySize {
		return fmt.Errorf("invalid x25519 private key length: %d", len(keys.X25519PrivateKey))
	}
	return nil
}

func normalizeBroker(broker RuntimeBroker) (RuntimeBroker, error) {
	endpoint := strings.TrimSpace(broker.Endpoint)
	if endpoint == "" {
		return RuntimeBroker{}, fmt.Errorf("empty runtime broker endpoint")
	}
	return RuntimeBroker{Endpoint: endpoint}, nil
}

func normalizeSnapshot(snapshot RosterSnapshot) (RosterSnapshot, error) {
	entries := make([]RosterEntry, 0, len(snapshot.Entries))
	seen := make(map[string]struct{}, len(snapshot.Entries))

	for i, entry := range snapshot.Entries {
		peerID, err := wire.CanonicalizePeerID(entry.PeerID)
		if err != nil {
			return RosterSnapshot{}, fmt.Errorf("roster entry %d peer_id: %w", i, err)
		}
		if len(entry.MemberCredential) == 0 {
			return RosterSnapshot{}, fmt.Errorf("roster entry %d member credential is required", i)
		}
		if _, ok := seen[peerID]; ok {
			return RosterSnapshot{}, fmt.Errorf("duplicate roster entry peer_id: %s", peerID)
		}
		seen[peerID] = struct{}{}

		entries = append(entries, RosterEntry{
			PeerID:           peerID,
			MemberCredential: append([]byte(nil), entry.MemberCredential...),
			DeviceName:       strings.TrimSpace(entry.DeviceName),
			Platform:         strings.TrimSpace(entry.Platform),
		})
	}

	return RosterSnapshot{Entries: entries}, nil
}

func normalizeJoinedBootstrap(joined JoinedBootstrap) (JoinedBootstrap, error) {
	networkID, err := wire.CanonicalizeNetworkID(joined.NetworkID)
	if err != nil {
		return JoinedBootstrap{}, fmt.Errorf("canonicalize network_id: %w", err)
	}
	if len(joined.SelfMemberCredential) == 0 {
		return JoinedBootstrap{}, fmt.Errorf("self member credential is required")
	}
	if len(joined.MailboxSecret) != mailboxSecretSize {
		return JoinedBootstrap{}, fmt.Errorf("invalid mailbox secret length: %d", len(joined.MailboxSecret))
	}

	broker, err := normalizeBroker(joined.RuntimeBroker)
	if err != nil {
		return JoinedBootstrap{}, err
	}
	snapshot, err := normalizeSnapshot(joined.RosterSnapshot)
	if err != nil {
		return JoinedBootstrap{}, err
	}

	return JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: append([]byte(nil), joined.SelfMemberCredential...),
		MailboxSecret:        append([]byte(nil), joined.MailboxSecret...),
		RuntimeBroker:        broker,
		RosterSnapshot:       snapshot,
	}, nil
}
