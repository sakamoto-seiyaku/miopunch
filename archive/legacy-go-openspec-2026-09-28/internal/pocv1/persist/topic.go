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
	"crypto/hkdf"
	"crypto/sha256"
	"encoding/base32"
	"fmt"
	"strings"

	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

const (
	inboxTopicInfoPrefix = "miopunch/v1/topic.inbox/"
	netRootInfo          = "miopunch/v1/net_root"
)

var base32RawNoPad = base32.StdEncoding.WithPadding(base32.NoPadding)

// TopicScope derives current v1 presence and inbox topics for one joined
// network.
type TopicScope struct {
	NetRoot       string
	mailboxSecret []byte
	salt          []byte
}

func newTopicScope(networkID string, mailboxSecret []byte) (TopicScope, error) {
	canonicalNetworkID, err := wire.CanonicalizeNetworkID(networkID)
	if err != nil {
		return TopicScope{}, fmt.Errorf("canonicalize network_id: %w", err)
	}
	if len(mailboxSecret) != mailboxSecretSize {
		return TopicScope{}, fmt.Errorf("invalid mailbox secret length: %d", len(mailboxSecret))
	}

	rawNetworkID, err := wire.DecodeNetworkID(canonicalNetworkID)
	if err != nil {
		return TopicScope{}, fmt.Errorf("decode network_id: %w", err)
	}

	sum := sha256.Sum256(rawNetworkID)
	salt := append([]byte(nil), sum[:wire.RawIDLen]...)
	rootMaterial, err := hkdf.Key(
		sha256.New,
		mailboxSecret,
		salt,
		netRootInfo,
		wire.RawIDLen,
	)
	if err != nil {
		return TopicScope{}, fmt.Errorf("derive net_root: %w", err)
	}

	return TopicScope{
		NetRoot:       strings.ToLower(base32RawNoPad.EncodeToString(rootMaterial)),
		mailboxSecret: append([]byte(nil), mailboxSecret...),
		salt:          salt,
	}, nil
}

// PresenceTopic returns the current v1 presence topic for peerID.
func (s TopicScope) PresenceTopic(peerID string) (string, error) {
	canonicalPeerID, err := wire.CanonicalizePeerID(peerID)
	if err != nil {
		return "", fmt.Errorf("canonicalize peer_id: %w", err)
	}
	return "mp/v1/net/" + s.NetRoot + "/presence/" + canonicalPeerID, nil
}

// InboxTopic returns the current v1 inbox topic for peerID.
func (s TopicScope) InboxTopic(peerID string) (string, error) {
	canonicalPeerID, err := wire.CanonicalizePeerID(peerID)
	if err != nil {
		return "", fmt.Errorf("canonicalize peer_id: %w", err)
	}

	inboxMaterial, err := hkdf.Key(
		sha256.New,
		s.mailboxSecret,
		s.salt,
		inboxTopicInfoPrefix+canonicalPeerID,
		wire.RawIDLen,
	)
	if err != nil {
		return "", fmt.Errorf("derive inbox topic: %w", err)
	}

	inbox := strings.ToLower(base32RawNoPad.EncodeToString(inboxMaterial))
	return "mp/v1/net/" + s.NetRoot + "/inbox/" + inbox, nil
}
