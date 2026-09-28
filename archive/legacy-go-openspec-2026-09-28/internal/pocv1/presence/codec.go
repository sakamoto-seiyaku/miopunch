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
	"encoding/json"
	"fmt"
	"strings"

	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

type payload struct {
	Version    int    `json:"v"`
	State      string `json:"state"`
	PeerID     string `json:"peer_id"`
	DeviceName string `json:"device_name"`
	Platform   string `json:"platform"`
	AppVer     string `json:"app_ver"`
	Timestamp  uint64 `json:"ts_unix_ms"`
}

// PresenceWildcardTopic returns the fixed wildcard subscription topic for one
// joined current v1 network.
func PresenceWildcardTopic(scope persist.TopicScope) string {
	return "mp/v1/net/" + strings.TrimSpace(scope.NetRoot) + "/presence/+"
}

// EncodeObservationPayload encodes one fixed current v1 JSON presence payload.
func EncodeObservationPayload(obs Observation) ([]byte, error) {
	canonicalPeerID, err := wire.CanonicalizePeerID(obs.PeerID)
	if err != nil {
		return nil, fmt.Errorf("canonicalize peer_id: %w", err)
	}
	if !obs.OnlineState.IsValid() {
		return nil, fmt.Errorf("invalid online_state: %q", obs.OnlineState)
	}

	body := payload{
		Version:    PayloadVersion,
		State:      string(obs.OnlineState),
		PeerID:     canonicalPeerID,
		DeviceName: strings.TrimSpace(obs.DeviceName),
		Platform:   strings.TrimSpace(obs.Platform),
		AppVer:     strings.TrimSpace(obs.AppVer),
		Timestamp:  obs.LastObservedUnixMs,
	}
	return json.Marshal(body)
}

// EncodeSelfPayload encodes the local peer's fixed current v1 payload.
func EncodeSelfPayload(cfg Config, state OnlineState, observedAtUnixMs uint64) ([]byte, error) {
	normalizedCfg, err := normalizeConfig(cfg)
	if err != nil {
		return nil, err
	}
	return EncodeObservationPayload(Observation{
		PeerID:             normalizedCfg.SelfPeerID,
		OnlineState:        state,
		DeviceName:         normalizedCfg.DeviceName,
		Platform:           normalizedCfg.Platform,
		AppVer:             normalizedCfg.AppVer,
		LastObservedUnixMs: observedAtUnixMs,
	})
}

// ParseObservation parses one topic/payload pair into a normalized observation.
//
// Invalid or out-of-scope inputs return a typed diagnostic and do not mutate
// discover state.
func ParseObservation(
	topic string,
	rawPayload []byte,
	expectedScope persist.TopicScope,
) (Observation, *Diagnostic) {
	topicPeerID, diag := parseTopic(topic, expectedScope)
	if diag != nil {
		return Observation{}, diag
	}

	var body payload
	if err := json.Unmarshal(rawPayload, &body); err != nil {
		return Observation{}, &Diagnostic{
			Kind:    DiagnosticMalformedJSON,
			Topic:   topic,
			PeerID:  topicPeerID,
			Message: fmt.Sprintf("decode JSON payload: %v", err),
		}
	}

	if body.Version != PayloadVersion {
		return Observation{}, &Diagnostic{
			Kind:    DiagnosticUnsupportedVersion,
			Topic:   topic,
			PeerID:  topicPeerID,
			Message: fmt.Sprintf("unsupported payload version: %d", body.Version),
		}
	}

	payloadPeerID, err := wire.CanonicalizePeerID(body.PeerID)
	if err != nil {
		return Observation{}, &Diagnostic{
			Kind:    DiagnosticInvalidPeerID,
			Topic:   topic,
			PeerID:  topicPeerID,
			Message: fmt.Sprintf("canonicalize payload peer_id: %v", err),
		}
	}

	if payloadPeerID != topicPeerID {
		return Observation{}, &Diagnostic{
			Kind:    DiagnosticTopicMismatch,
			Topic:   topic,
			PeerID:  topicPeerID,
			Message: fmt.Sprintf("payload peer_id %q does not match topic peer_id %q", payloadPeerID, topicPeerID),
		}
	}

	state := OnlineState(strings.TrimSpace(body.State))
	if !state.IsValid() {
		return Observation{}, &Diagnostic{
			Kind:    DiagnosticInvalidState,
			Topic:   topic,
			PeerID:  topicPeerID,
			Message: fmt.Sprintf("invalid state %q", body.State),
		}
	}

	return Observation{
		PeerID:             payloadPeerID,
		OnlineState:        state,
		DeviceName:         strings.TrimSpace(body.DeviceName),
		Platform:           strings.TrimSpace(body.Platform),
		AppVer:             strings.TrimSpace(body.AppVer),
		LastObservedUnixMs: body.Timestamp,
	}, nil
}

func parseTopic(topic string, expectedScope persist.TopicScope) (string, *Diagnostic) {
	parts := strings.Split(strings.TrimSpace(topic), "/")
	if len(parts) != 6 ||
		parts[0] != "mp" ||
		parts[1] != "v1" ||
		parts[2] != "net" ||
		parts[4] != "presence" ||
		strings.TrimSpace(parts[3]) == "" ||
		strings.TrimSpace(parts[5]) == "" {
		return "", &Diagnostic{
			Kind:    DiagnosticTopicMismatch,
			Topic:   topic,
			Message: "topic does not match mp/v1/net/<net_root>/presence/<peer_id>",
		}
	}

	if parts[3] != strings.TrimSpace(expectedScope.NetRoot) {
		return "", &Diagnostic{
			Kind:    DiagnosticTopicMismatch,
			Topic:   topic,
			Message: fmt.Sprintf("topic net_root %q does not match expected %q", parts[3], strings.TrimSpace(expectedScope.NetRoot)),
		}
	}

	peerID, err := wire.CanonicalizePeerID(parts[5])
	if err != nil {
		return "", &Diagnostic{
			Kind:    DiagnosticTopicMismatch,
			Topic:   topic,
			Message: fmt.Sprintf("canonicalize topic peer_id: %v", err),
		}
	}
	return peerID, nil
}
