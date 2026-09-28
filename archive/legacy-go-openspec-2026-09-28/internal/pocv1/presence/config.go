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

package presence

import (
	"fmt"
	"strings"

	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

// Config is the fixed current v1 presence/discover runtime input set.
type Config struct {
	NetworkID      string
	RuntimeBroker  persist.RuntimeBroker
	TopicScope     persist.TopicScope
	RosterSnapshot persist.RosterSnapshot
	SelfPeerID     string
	DeviceName     string
	Platform       string
	AppVer         string
}

// LoadConfig loads the fixed current v1 presence inputs from persistence and
// combines them with caller-supplied local display hints.
func LoadConfig(
	store *persist.Store,
	networkID string,
	deviceName string,
	platform string,
	appVer string,
) (Config, error) {
	if store == nil {
		return Config{}, fmt.Errorf("nil persistence store")
	}

	canonicalNetworkID, err := wire.CanonicalizeNetworkID(networkID)
	if err != nil {
		return Config{}, fmt.Errorf("canonicalize network_id: %w", err)
	}

	runtimeBroker, err := store.LoadRuntimeBroker(canonicalNetworkID)
	if err != nil {
		return Config{}, err
	}

	topicScope, err := store.LoadTopicScope(canonicalNetworkID)
	if err != nil {
		return Config{}, err
	}

	rosterSnapshot, err := store.LoadRosterSnapshot(canonicalNetworkID)
	if err != nil {
		return Config{}, err
	}

	deviceKeys, err := store.LoadDeviceKeys()
	if err != nil {
		return Config{}, err
	}

	selfPeerID, err := deviceKeys.PeerID()
	if err != nil {
		return Config{}, err
	}

	cfg := Config{
		NetworkID:      canonicalNetworkID,
		RuntimeBroker:  runtimeBroker,
		TopicScope:     topicScope,
		RosterSnapshot: rosterSnapshot,
		SelfPeerID:     selfPeerID,
		DeviceName:     strings.TrimSpace(deviceName),
		Platform:       strings.TrimSpace(platform),
		AppVer:         strings.TrimSpace(appVer),
	}

	return normalizeConfig(cfg)
}

func normalizeConfig(cfg Config) (Config, error) {
	canonicalNetworkID, err := wire.CanonicalizeNetworkID(cfg.NetworkID)
	if err != nil {
		return Config{}, fmt.Errorf("canonicalize network_id: %w", err)
	}

	canonicalSelfPeerID, err := wire.CanonicalizePeerID(cfg.SelfPeerID)
	if err != nil {
		return Config{}, fmt.Errorf("canonicalize self peer_id: %w", err)
	}

	endpoint := strings.TrimSpace(cfg.RuntimeBroker.Endpoint)
	if endpoint == "" {
		return Config{}, fmt.Errorf("empty runtime broker endpoint")
	}

	if strings.TrimSpace(cfg.TopicScope.NetRoot) == "" {
		return Config{}, fmt.Errorf("empty topic scope net_root")
	}

	if _, err := cfg.TopicScope.PresenceTopic(canonicalSelfPeerID); err != nil {
		return Config{}, err
	}

	entries := make([]persist.RosterEntry, 0, len(cfg.RosterSnapshot.Entries))
	seen := make(map[string]struct{}, len(cfg.RosterSnapshot.Entries))
	for i, entry := range cfg.RosterSnapshot.Entries {
		peerID, err := wire.CanonicalizePeerID(entry.PeerID)
		if err != nil {
			return Config{}, fmt.Errorf("roster entry %d peer_id: %w", i, err)
		}
		if _, ok := seen[peerID]; ok {
			return Config{}, fmt.Errorf("duplicate roster entry peer_id: %s", peerID)
		}
		seen[peerID] = struct{}{}

		entries = append(entries, persist.RosterEntry{
			PeerID:           peerID,
			MemberCredential: append([]byte(nil), entry.MemberCredential...),
			DeviceName:       strings.TrimSpace(entry.DeviceName),
			Platform:         strings.TrimSpace(entry.Platform),
		})
	}

	return Config{
		NetworkID: canonicalNetworkID,
		RuntimeBroker: persist.RuntimeBroker{
			Endpoint: endpoint,
		},
		TopicScope: cfg.TopicScope,
		RosterSnapshot: persist.RosterSnapshot{
			Entries: entries,
		},
		SelfPeerID: canonicalSelfPeerID,
		DeviceName: strings.TrimSpace(cfg.DeviceName),
		Platform:   strings.TrimSpace(cfg.Platform),
		AppVer:     strings.TrimSpace(cfg.AppVer),
	}, nil
}

func brokerURL(endpoint string) string {
	trimmed := strings.TrimSpace(endpoint)
	if trimmed == "" {
		return ""
	}
	if strings.Contains(trimmed, "://") {
		return trimmed
	}
	return "tcp://" + trimmed
}
