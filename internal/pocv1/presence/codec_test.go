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
	"bytes"
	"strings"
	"testing"

	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

func TestLoadConfigReadsPersistedInputs(t *testing.T) {
	store, joined, expectedSelfPeerID := mustPersistedStore(t, "broker.example.net:1883")

	cfg, err := LoadConfig(store, joined.NetworkID, "living room", "linux", "1.2.3")
	if err != nil {
		t.Fatalf("LoadConfig() error = %v, want nil", err)
	}

	if cfg.RuntimeBroker.Endpoint != joined.RuntimeBroker.Endpoint {
		t.Fatalf("LoadConfig().RuntimeBroker.Endpoint = %q, want %q", cfg.RuntimeBroker.Endpoint, joined.RuntimeBroker.Endpoint)
	}
	if cfg.NetworkID != joined.NetworkID {
		t.Fatalf("LoadConfig().NetworkID = %q, want %q", cfg.NetworkID, joined.NetworkID)
	}
	if cfg.SelfPeerID != expectedSelfPeerID {
		t.Fatalf("LoadConfig().SelfPeerID = %q, want %q", cfg.SelfPeerID, expectedSelfPeerID)
	}
	if cfg.TopicScope.NetRoot == "" {
		t.Fatalf("LoadConfig().TopicScope.NetRoot = empty, want non-empty")
	}
	if cfg.DeviceName != "living room" {
		t.Fatalf("LoadConfig().DeviceName = %q, want %q", cfg.DeviceName, "living room")
	}
	if cfg.Platform != "linux" {
		t.Fatalf("LoadConfig().Platform = %q, want %q", cfg.Platform, "linux")
	}
	if cfg.AppVer != "1.2.3" {
		t.Fatalf("LoadConfig().AppVer = %q, want %q", cfg.AppVer, "1.2.3")
	}
}

func TestEncodeSelfPayloadRoundTrips(t *testing.T) {
	store, joined, _ := mustPersistedStore(t, "broker.example.net:1883")
	cfg, err := LoadConfig(store, joined.NetworkID, "alpha", "linux", "9.9.9")
	if err != nil {
		t.Fatalf("LoadConfig() error = %v, want nil", err)
	}

	rawPayload, err := EncodeSelfPayload(cfg, OnlineStateOnline, 1234)
	if err != nil {
		t.Fatalf("EncodeSelfPayload() error = %v, want nil", err)
	}

	topic, err := cfg.TopicScope.PresenceTopic(cfg.SelfPeerID)
	if err != nil {
		t.Fatalf("PresenceTopic(self) error = %v, want nil", err)
	}

	got, diag := ParseObservation(topic, rawPayload, cfg.TopicScope)
	if diag != nil {
		t.Fatalf("ParseObservation() diagnostic = %+v, want nil", diag)
	}

	if got.PeerID != cfg.SelfPeerID {
		t.Fatalf("ParseObservation().PeerID = %q, want %q", got.PeerID, cfg.SelfPeerID)
	}
	if got.OnlineState != OnlineStateOnline {
		t.Fatalf("ParseObservation().OnlineState = %q, want %q", got.OnlineState, OnlineStateOnline)
	}
	if got.DeviceName != "alpha" {
		t.Fatalf("ParseObservation().DeviceName = %q, want %q", got.DeviceName, "alpha")
	}
	if got.Platform != "linux" {
		t.Fatalf("ParseObservation().Platform = %q, want %q", got.Platform, "linux")
	}
	if got.AppVer != "9.9.9" {
		t.Fatalf("ParseObservation().AppVer = %q, want %q", got.AppVer, "9.9.9")
	}
	if got.LastObservedUnixMs != 1234 {
		t.Fatalf("ParseObservation().LastObservedUnixMs = %d, want %d", got.LastObservedUnixMs, 1234)
	}
}

func TestParseObservationRejectsInvalidInputs(t *testing.T) {
	store, joined, _ := mustPersistedStore(t, "broker.example.net:1883")
	cfg, err := LoadConfig(store, joined.NetworkID, "alpha", "linux", "9.9.9")
	if err != nil {
		t.Fatalf("LoadConfig() error = %v, want nil", err)
	}

	topic, err := cfg.TopicScope.PresenceTopic(cfg.SelfPeerID)
	if err != nil {
		t.Fatalf("PresenceTopic(self) error = %v, want nil", err)
	}

	otherPeerID := mustPeerID(t, 0x66)
	otherTopic, err := cfg.TopicScope.PresenceTopic(otherPeerID)
	if err != nil {
		t.Fatalf("PresenceTopic(other) error = %v, want nil", err)
	}

	cases := []struct {
		name     string
		topic    string
		payload  []byte
		wantKind DiagnosticKind
	}{
		{
			name:     "malformed_json",
			topic:    topic,
			payload:  []byte("{"),
			wantKind: DiagnosticMalformedJSON,
		},
		{
			name:     "unsupported_version",
			topic:    topic,
			payload:  []byte(`{"v":2,"state":"online","peer_id":"` + cfg.SelfPeerID + `","device_name":"x","platform":"y","app_ver":"","ts_unix_ms":1}`),
			wantKind: DiagnosticUnsupportedVersion,
		},
		{
			name:     "invalid_peer_id",
			topic:    topic,
			payload:  []byte(`{"v":1,"state":"online","peer_id":"bad","device_name":"x","platform":"y","app_ver":"","ts_unix_ms":1}`),
			wantKind: DiagnosticInvalidPeerID,
		},
		{
			name:     "topic_payload_mismatch",
			topic:    topic,
			payload:  []byte(`{"v":1,"state":"online","peer_id":"` + otherPeerID + `","device_name":"x","platform":"y","app_ver":"","ts_unix_ms":1}`),
			wantKind: DiagnosticTopicMismatch,
		},
		{
			name:     "unexpected_scope",
			topic:    stringsReplaceOne(otherTopic, cfg.TopicScope.NetRoot, cfg.TopicScope.NetRoot+"x"),
			payload:  []byte(`{"v":1,"state":"online","peer_id":"` + otherPeerID + `","device_name":"x","platform":"y","app_ver":"","ts_unix_ms":1}`),
			wantKind: DiagnosticTopicMismatch,
		},
	}

	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			got, diag := ParseObservation(tt.topic, tt.payload, cfg.TopicScope)
			if diag == nil {
				t.Fatalf("ParseObservation(%q) observation = %+v, want diagnostic", tt.topic, got)
			}
			if diag.Kind != tt.wantKind {
				t.Fatalf("ParseObservation(%q) diagnostic kind = %q, want %q", tt.topic, diag.Kind, tt.wantKind)
			}
		})
	}
}

func mustPersistedStore(t *testing.T, brokerEndpoint string) (*persist.Store, persist.JoinedBootstrap, string) {
	t.Helper()

	store, err := persist.Open(t.TempDir())
	if err != nil {
		t.Fatalf("persist.Open() error = %v, want nil", err)
	}

	keys, err := store.EnsureDeviceKeys()
	if err != nil {
		t.Fatalf("EnsureDeviceKeys() error = %v, want nil", err)
	}
	selfPeerID, err := keys.PeerID()
	if err != nil {
		t.Fatalf("DeviceKeys.PeerID() error = %v, want nil", err)
	}

	networkID, err := wire.EncodeNetworkID(bytes.Repeat([]byte{0x5a}, wire.RawIDLen))
	if err != nil {
		t.Fatalf("EncodeNetworkID() error = %v, want nil", err)
	}

	joined := persist.JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: []byte("self-member-credential"),
		MailboxSecret:        bytes.Repeat([]byte{0x33}, 32),
		RuntimeBroker: persist.RuntimeBroker{
			Endpoint: brokerEndpoint,
		},
		RosterSnapshot: persist.RosterSnapshot{
			Entries: []persist.RosterEntry{
				{
					PeerID:           mustPeerID(t, 0x11),
					MemberCredential: []byte("credential-a"),
					DeviceName:       "alpha",
					Platform:         "linux",
				},
				{
					PeerID:           mustPeerID(t, 0x22),
					MemberCredential: []byte("credential-b"),
					DeviceName:       "beta",
					Platform:         "windows",
				},
			},
		},
	}

	if err := store.PersistJoinedBootstrap(joined); err != nil {
		t.Fatalf("PersistJoinedBootstrap() error = %v, want nil", err)
	}

	return store, joined, selfPeerID
}

func mustPeerID(t *testing.T, seedByte byte) string {
	t.Helper()

	pub := bytes.Repeat([]byte{seedByte}, 32)
	peerID, err := wire.PeerIDFromEd25519Pub(pub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(%x) error = %v, want nil", pub, err)
	}
	return peerID
}

func stringsReplaceOne(s string, old string, new string) string {
	return strings.Replace(s, old, new, 1)
}
