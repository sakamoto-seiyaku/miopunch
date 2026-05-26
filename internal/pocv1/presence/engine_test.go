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
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/pocv1/persist"
)

func TestEngineBuildsRosterBoundedDiscoverView(t *testing.T) {
	cfg := mustConfigWithRoster(t, "broker.example.net:1883", func(cfg *Config) {
		cfg.RosterSnapshot.Entries = []persist.RosterEntry{
			{
				PeerID:           mustPeerID(t, 0x11),
				MemberCredential: []byte("credential-a"),
				DeviceName:       "roster-name",
				Platform:         "",
			},
			{
				PeerID:           cfg.SelfPeerID,
				MemberCredential: []byte("credential-self"),
				DeviceName:       "self",
				Platform:         "linux",
			},
		}
	})

	engine, err := NewEngine(cfg)
	if err != nil {
		t.Fatalf("NewEngine() error = %v, want nil", err)
	}

	remotePeerID := cfg.RosterSnapshot.Entries[0].PeerID
	remoteTopic, err := cfg.TopicScope.PresenceTopic(remotePeerID)
	if err != nil {
		t.Fatalf("PresenceTopic(remote) error = %v, want nil", err)
	}
	remotePayload, err := EncodeObservationPayload(Observation{
		PeerID:             remotePeerID,
		OnlineState:        OnlineStateOnline,
		DeviceName:         "presence-name",
		Platform:           "windows",
		AppVer:             "2.3.4",
		LastObservedUnixMs: 1234,
	})
	if err != nil {
		t.Fatalf("EncodeObservationPayload(remote) error = %v, want nil", err)
	}
	if !engine.ApplyMessage(remoteTopic, remotePayload) {
		t.Fatalf("ApplyMessage(remote) = false, want true")
	}

	unknownPeerID := mustPeerID(t, 0x99)
	unknownTopic, err := cfg.TopicScope.PresenceTopic(unknownPeerID)
	if err != nil {
		t.Fatalf("PresenceTopic(unknown) error = %v, want nil", err)
	}
	unknownPayload, err := EncodeObservationPayload(Observation{
		PeerID:             unknownPeerID,
		OnlineState:        OnlineStateOnline,
		DeviceName:         "stranger",
		Platform:           "ios",
		LastObservedUnixMs: 9999,
	})
	if err != nil {
		t.Fatalf("EncodeObservationPayload(unknown) error = %v, want nil", err)
	}
	if !engine.ApplyMessage(unknownTopic, unknownPayload) {
		t.Fatalf("ApplyMessage(unknown) = false, want true")
	}

	got := engine.View(time.UnixMilli(5000))
	if got.NetworkID != cfg.NetworkID {
		t.Fatalf("View().NetworkID = %q, want %q", got.NetworkID, cfg.NetworkID)
	}
	if got.SelfPeerID != cfg.SelfPeerID {
		t.Fatalf("View().SelfPeerID = %q, want %q", got.SelfPeerID, cfg.SelfPeerID)
	}
	if len(got.Peers) != 1 {
		t.Fatalf("View().Peers length = %d, want 1", len(got.Peers))
	}

	peer := got.Peers[0]
	if peer.PeerID != remotePeerID {
		t.Fatalf("View().Peers[0].PeerID = %q, want %q", peer.PeerID, remotePeerID)
	}
	if peer.OnlineState != OnlineStateOnline {
		t.Fatalf("View().Peers[0].OnlineState = %q, want %q", peer.OnlineState, OnlineStateOnline)
	}
	if peer.DeviceName != "roster-name" {
		t.Fatalf("View().Peers[0].DeviceName = %q, want %q", peer.DeviceName, "roster-name")
	}
	if peer.Platform != "windows" {
		t.Fatalf("View().Peers[0].Platform = %q, want %q", peer.Platform, "windows")
	}
	if peer.AppVer != "2.3.4" {
		t.Fatalf("View().Peers[0].AppVer = %q, want %q", peer.AppVer, "2.3.4")
	}
	if peer.LastObservedUnixMs != 1234 {
		t.Fatalf("View().Peers[0].LastObservedUnixMs = %d, want %d", peer.LastObservedUnixMs, 1234)
	}

	diagnostics := engine.Diagnostics()
	if len(diagnostics) != 1 {
		t.Fatalf("Diagnostics() length = %d, want 1", len(diagnostics))
	}
	if diagnostics[0].Kind != DiagnosticUnknownPeer {
		t.Fatalf("Diagnostics()[0].Kind = %q, want %q", diagnostics[0].Kind, DiagnosticUnknownPeer)
	}

	lastSeen := indexLastSeen(engine.LastSeen())
	if lastSeen[remotePeerID].LastOnlineUnixMs != 1234 {
		t.Fatalf("LastSeen()[remote].LastOnlineUnixMs = %d, want %d", lastSeen[remotePeerID].LastOnlineUnixMs, 1234)
	}
	if lastSeen[unknownPeerID].LastState != OnlineStateOnline {
		t.Fatalf("LastSeen()[unknown].LastState = %q, want %q", lastSeen[unknownPeerID].LastState, OnlineStateOnline)
	}
}

func TestEngineRosterOnlyOfflineAndDuplicateConvergence(t *testing.T) {
	cfg := mustConfigWithRoster(t, "broker.example.net:1883", nil)
	engine, err := NewEngine(cfg)
	if err != nil {
		t.Fatalf("NewEngine() error = %v, want nil", err)
	}

	view := engine.View(time.UnixMilli(1))
	wantPeers := 0
	for _, entry := range cfg.RosterSnapshot.Entries {
		if entry.PeerID == cfg.SelfPeerID {
			continue
		}
		wantPeers++
	}
	if len(view.Peers) != wantPeers {
		t.Fatalf("View().Peers length = %d, want %d", len(view.Peers), wantPeers)
	}
	for _, peer := range view.Peers {
		if peer.OnlineState != OnlineStateOffline {
			t.Fatalf("View().Peers[%q].OnlineState = %q, want %q", peer.PeerID, peer.OnlineState, OnlineStateOffline)
		}
	}

	remotePeerID := cfg.RosterSnapshot.Entries[0].PeerID
	remoteTopic, err := cfg.TopicScope.PresenceTopic(remotePeerID)
	if err != nil {
		t.Fatalf("PresenceTopic(remote) error = %v, want nil", err)
	}
	rawPayload, err := EncodeObservationPayload(Observation{
		PeerID:             remotePeerID,
		OnlineState:        OnlineStateOnline,
		DeviceName:         "alpha",
		Platform:           "linux",
		LastObservedUnixMs: 1234,
	})
	if err != nil {
		t.Fatalf("EncodeObservationPayload(remote) error = %v, want nil", err)
	}

	if !engine.ApplyMessage(remoteTopic, rawPayload) {
		t.Fatalf("ApplyMessage(first) = false, want true")
	}
	if engine.ApplyMessage(remoteTopic, rawPayload) {
		t.Fatalf("ApplyMessage(duplicate) = true, want false")
	}
}

func TestEngineTreatsSelfObservationAsLocalOnly(t *testing.T) {
	cfg := mustConfigWithRoster(t, "broker.example.net:1883", func(cfg *Config) {
		cfg.RosterSnapshot.Entries = []persist.RosterEntry{
			{
				PeerID:           mustPeerID(t, 0x11),
				MemberCredential: []byte("credential-a"),
				DeviceName:       "alpha",
				Platform:         "linux",
			},
		}
	})
	engine, err := NewEngine(cfg)
	if err != nil {
		t.Fatalf("NewEngine() error = %v, want nil", err)
	}

	selfTopic, err := cfg.TopicScope.PresenceTopic(cfg.SelfPeerID)
	if err != nil {
		t.Fatalf("PresenceTopic(self) error = %v, want nil", err)
	}
	selfPayload, err := EncodeObservationPayload(Observation{
		PeerID:             cfg.SelfPeerID,
		OnlineState:        OnlineStateOnline,
		DeviceName:         "local-name",
		Platform:           "linux",
		LastObservedUnixMs: 77,
	})
	if err != nil {
		t.Fatalf("EncodeObservationPayload(self) error = %v, want nil", err)
	}
	if !engine.ApplyMessage(selfTopic, selfPayload) {
		t.Fatalf("ApplyMessage(self) = false, want true")
	}

	if got := engine.Diagnostics(); len(got) != 0 {
		t.Fatalf("Diagnostics() = %+v, want empty", got)
	}

	view := engine.View(time.UnixMilli(88))
	if len(view.Peers) != 1 {
		t.Fatalf("View().Peers length = %d, want 1", len(view.Peers))
	}
	if view.Peers[0].PeerID == cfg.SelfPeerID {
		t.Fatalf("View().Peers[0].PeerID = %q, want remote peer", view.Peers[0].PeerID)
	}
	if view.Peers[0].OnlineState != OnlineStateOffline {
		t.Fatalf("View().Peers[0].OnlineState = %q, want %q", view.Peers[0].OnlineState, OnlineStateOffline)
	}

	lastSeen := indexLastSeen(engine.LastSeen())
	if lastSeen[cfg.SelfPeerID].LastState != OnlineStateOnline {
		t.Fatalf("LastSeen()[self].LastState = %q, want %q", lastSeen[cfg.SelfPeerID].LastState, OnlineStateOnline)
	}
}

func TestEngineApplyMessageInvalidObservationDoesNotMutateDiscoverState(t *testing.T) {
	cfg := mustConfigWithRoster(t, "broker.example.net:1883", nil)

	remotePeerID := cfg.RosterSnapshot.Entries[0].PeerID
	remoteTopic, err := cfg.TopicScope.PresenceTopic(remotePeerID)
	if err != nil {
		t.Fatalf("PresenceTopic(remote) error = %v, want nil", err)
	}
	seedPayload, err := EncodeObservationPayload(Observation{
		PeerID:             remotePeerID,
		OnlineState:        OnlineStateOnline,
		DeviceName:         "alpha",
		Platform:           "linux",
		AppVer:             "1.0.0",
		LastObservedUnixMs: 1234,
	})
	if err != nil {
		t.Fatalf("EncodeObservationPayload(seed) error = %v, want nil", err)
	}

	otherPeerID := mustPeerID(t, 0x66)
	otherTopic, err := cfg.TopicScope.PresenceTopic(otherPeerID)
	if err != nil {
		t.Fatalf("PresenceTopic(other) error = %v, want nil", err)
	}

	cases := []struct {
		name         string
		topic        string
		payload      []byte
		wantDiagKind DiagnosticKind
	}{
		{
			name:         "malformed_json",
			topic:        remoteTopic,
			payload:      []byte("{"),
			wantDiagKind: DiagnosticMalformedJSON,
		},
		{
			name:         "unsupported_version",
			topic:        remoteTopic,
			payload:      []byte(fmt.Sprintf(`{"v":2,"state":"online","peer_id":"%s","device_name":"x","platform":"y","app_ver":"","ts_unix_ms":1}`, remotePeerID)),
			wantDiagKind: DiagnosticUnsupportedVersion,
		},
		{
			name:         "invalid_peer_id",
			topic:        remoteTopic,
			payload:      []byte(`{"v":1,"state":"online","peer_id":"bad","device_name":"x","platform":"y","app_ver":"","ts_unix_ms":1}`),
			wantDiagKind: DiagnosticInvalidPeerID,
		},
		{
			name:         "topic_payload_mismatch",
			topic:        remoteTopic,
			payload:      []byte(fmt.Sprintf(`{"v":1,"state":"online","peer_id":"%s","device_name":"x","platform":"y","app_ver":"","ts_unix_ms":1}`, otherPeerID)),
			wantDiagKind: DiagnosticTopicMismatch,
		},
		{
			name:         "unexpected_scope",
			topic:        stringsReplaceOne(otherTopic, cfg.TopicScope.NetRoot, cfg.TopicScope.NetRoot+"x"),
			payload:      []byte(fmt.Sprintf(`{"v":1,"state":"online","peer_id":"%s","device_name":"x","platform":"y","app_ver":"","ts_unix_ms":1}`, otherPeerID)),
			wantDiagKind: DiagnosticTopicMismatch,
		},
	}

	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			engine, err := NewEngine(cfg)
			if err != nil {
				t.Fatalf("NewEngine() error = %v, want nil", err)
			}
			if !engine.ApplyMessage(remoteTopic, seedPayload) {
				t.Fatalf("ApplyMessage(seed) = false, want true")
			}

			beforeView := engine.View(time.UnixMilli(5000))
			beforeObserved := snapshotObserved(engine.observed)
			beforeLastSeen := indexLastSeen(engine.LastSeen())
			beforeDiagLen := len(engine.Diagnostics())

			changed := engine.ApplyMessage(tt.topic, tt.payload)
			if !changed {
				t.Fatalf("ApplyMessage(%s) = false, want true", tt.name)
			}

			afterView := engine.View(time.UnixMilli(5000))
			if diff := diffDiscoverView(beforeView, afterView); diff != "" {
				t.Fatalf("ApplyMessage(%s) mutated DiscoverView (-before +after): %s", tt.name, diff)
			}
			if diff := diffObserved(beforeObserved, snapshotObserved(engine.observed)); diff != "" {
				t.Fatalf("ApplyMessage(%s) mutated observed set (-before +after): %s", tt.name, diff)
			}
			if diff := diffLastSeen(beforeLastSeen, indexLastSeen(engine.LastSeen())); diff != "" {
				t.Fatalf("ApplyMessage(%s) mutated LastSeen (-before +after): %s", tt.name, diff)
			}

			diagnostics := engine.Diagnostics()
			if len(diagnostics) != beforeDiagLen+1 {
				t.Fatalf("ApplyMessage(%s) Diagnostics() length = %d, want %d", tt.name, len(diagnostics), beforeDiagLen+1)
			}
			if diagnostics[len(diagnostics)-1].Kind != tt.wantDiagKind {
				t.Fatalf("ApplyMessage(%s) diagnostic kind = %q, want %q", tt.name, diagnostics[len(diagnostics)-1].Kind, tt.wantDiagKind)
			}
		})
	}
}

func TestDialOnlineSurfaceUsesOnlyDiscoverPeerOnlineState(t *testing.T) {
	view := DiscoverView{
		NetworkID:        "NET",
		SelfPeerID:       "SELF",
		ObservedAtUnixMs: 1234,
		Peers: []DiscoverPeer{
			{
				PeerID:             "PEER-A",
				OnlineState:        OnlineStateOnline,
				DeviceName:         "changed later",
				Platform:           "linux",
				AppVer:             "1",
				LastObservedUnixMs: 11,
			},
			{
				PeerID:             "PEER-B",
				OnlineState:        OnlineStateOffline,
				DeviceName:         "irrelevant",
				Platform:           "windows",
				AppVer:             "2",
				LastObservedUnixMs: 22,
			},
		},
	}

	got := DialOnlineSurface(view)
	if len(got) != 2 {
		t.Fatalf("DialOnlineSurface() length = %d, want 2", len(got))
	}
	if got["PEER-A"] != OnlineStateOnline {
		t.Fatalf("DialOnlineSurface()[PEER-A] = %q, want %q", got["PEER-A"], OnlineStateOnline)
	}
	if got["PEER-B"] != OnlineStateOffline {
		t.Fatalf("DialOnlineSurface()[PEER-B] = %q, want %q", got["PEER-B"], OnlineStateOffline)
	}
}

func TestProjectViewProjectsDiscoverViewWithoutRejoin(t *testing.T) {
	view := DiscoverView{
		NetworkID:        "NET",
		SelfPeerID:       "SELF",
		ObservedAtUnixMs: 99,
		Peers: []DiscoverPeer{
			{
				PeerID:             "PEER-A",
				OnlineState:        OnlineStateOnline,
				DeviceName:         "alpha",
				Platform:           "linux",
				AppVer:             "1.0",
				LastObservedUnixMs: 44,
			},
		},
	}

	got := ProjectView(view)
	if got.NetworkID != view.NetworkID {
		t.Fatalf("ProjectView(view).NetworkID = %q, want %q", got.NetworkID, view.NetworkID)
	}
	if got.SelfPeerID != view.SelfPeerID {
		t.Fatalf("ProjectView(view).SelfPeerID = %q, want %q", got.SelfPeerID, view.SelfPeerID)
	}
	if len(got.Peers) != 1 {
		t.Fatalf("ProjectView(view).Peers length = %d, want 1", len(got.Peers))
	}
	if got.Peers[0].PeerID != view.Peers[0].PeerID {
		t.Fatalf("ProjectView(view).Peers[0].PeerID = %q, want %q", got.Peers[0].PeerID, view.Peers[0].PeerID)
	}
	if got.Peers[0].OnlineState != view.Peers[0].OnlineState {
		t.Fatalf("ProjectView(view).Peers[0].OnlineState = %q, want %q", got.Peers[0].OnlineState, view.Peers[0].OnlineState)
	}
	if got.Peers[0].DeviceName != view.Peers[0].DeviceName {
		t.Fatalf("ProjectView(view).Peers[0].DeviceName = %q, want %q", got.Peers[0].DeviceName, view.Peers[0].DeviceName)
	}

	view.Peers[0].DeviceName = "mutated"
	if got.Peers[0].DeviceName != "alpha" {
		t.Fatalf("ProjectView(view) preserved DeviceName = %q, want %q", got.Peers[0].DeviceName, "alpha")
	}
}

func TestMergeLastSeenPreservesLastOnlineAcrossOfflineTransition(t *testing.T) {
	got := MergeLastSeen(LastSeenPeer{}, Observation{
		PeerID:             "PEER-A",
		OnlineState:        OnlineStateOnline,
		LastObservedUnixMs: 100,
	})
	if got.LastState != OnlineStateOnline {
		t.Fatalf("MergeLastSeen({}, online@100).LastState = %q, want %q", got.LastState, OnlineStateOnline)
	}
	if got.LastObservedUnixMs != 100 {
		t.Fatalf("MergeLastSeen({}, online@100).LastObservedUnixMs = %d, want %d", got.LastObservedUnixMs, 100)
	}
	if got.LastOnlineUnixMs != 100 {
		t.Fatalf("MergeLastSeen({}, online@100).LastOnlineUnixMs = %d, want %d", got.LastOnlineUnixMs, 100)
	}

	got = MergeLastSeen(got, Observation{
		PeerID:             "PEER-A",
		OnlineState:        OnlineStateOffline,
		LastObservedUnixMs: 200,
	})
	if got.LastState != OnlineStateOffline {
		t.Fatalf("MergeLastSeen(online@100, offline@200).LastState = %q, want %q", got.LastState, OnlineStateOffline)
	}
	if got.LastObservedUnixMs != 200 {
		t.Fatalf("MergeLastSeen(online@100, offline@200).LastObservedUnixMs = %d, want %d", got.LastObservedUnixMs, 200)
	}
	if got.LastOnlineUnixMs != 100 {
		t.Fatalf("MergeLastSeen(online@100, offline@200).LastOnlineUnixMs = %d, want %d", got.LastOnlineUnixMs, 100)
	}

	got = MergeLastSeen(got, Observation{
		PeerID:             "PEER-A",
		OnlineState:        OnlineStateOnline,
		LastObservedUnixMs: 300,
	})
	if got.LastState != OnlineStateOnline {
		t.Fatalf("MergeLastSeen(offline@200, online@300).LastState = %q, want %q", got.LastState, OnlineStateOnline)
	}
	if got.LastObservedUnixMs != 300 {
		t.Fatalf("MergeLastSeen(offline@200, online@300).LastObservedUnixMs = %d, want %d", got.LastObservedUnixMs, 300)
	}
	if got.LastOnlineUnixMs != 300 {
		t.Fatalf("MergeLastSeen(offline@200, online@300).LastOnlineUnixMs = %d, want %d", got.LastOnlineUnixMs, 300)
	}
}

func mustConfigWithRoster(t *testing.T, brokerEndpoint string, mutate func(*Config)) Config {
	t.Helper()

	store, joined, _ := mustPersistedStore(t, brokerEndpoint)
	cfg, err := LoadConfig(store, joined.NetworkID, "local-name", "linux", "1.0.0")
	if err != nil {
		t.Fatalf("LoadConfig() error = %v, want nil", err)
	}
	if mutate != nil {
		mutate(&cfg)
		cfg, err = normalizeConfig(cfg)
		if err != nil {
			t.Fatalf("normalizeConfig(mutated) error = %v, want nil", err)
		}
	}
	return cfg
}

func indexLastSeen(items []LastSeenPeer) map[string]LastSeenPeer {
	out := make(map[string]LastSeenPeer, len(items))
	for _, item := range items {
		out[item.PeerID] = item
	}
	return out
}

func snapshotObserved(in map[string]observationRecord) map[string]observationRecord {
	out := make(map[string]observationRecord, len(in))
	for peerID, record := range in {
		out[peerID] = observationRecord{
			obs:        record.obs,
			rawPayload: append([]byte(nil), record.rawPayload...),
		}
	}
	return out
}

func diffDiscoverView(before DiscoverView, after DiscoverView) string {
	if before.NetworkID != after.NetworkID ||
		before.SelfPeerID != after.SelfPeerID ||
		before.ObservedAtUnixMs != after.ObservedAtUnixMs ||
		len(before.Peers) != len(after.Peers) {
		return fmt.Sprintf("before=%+v after=%+v", before, after)
	}

	for i := range before.Peers {
		if before.Peers[i] != after.Peers[i] {
			return fmt.Sprintf("before=%+v after=%+v", before, after)
		}
	}

	return ""
}

func diffObserved(before map[string]observationRecord, after map[string]observationRecord) string {
	if len(before) != len(after) {
		return fmt.Sprintf("before=%d entries after=%d entries", len(before), len(after))
	}
	for peerID, beforeRecord := range before {
		afterRecord, ok := after[peerID]
		if !ok {
			return fmt.Sprintf("missing peer_id %q after mutation", peerID)
		}
		if beforeRecord.obs != afterRecord.obs {
			return fmt.Sprintf("peer_id %q observation before=%+v after=%+v", peerID, beforeRecord.obs, afterRecord.obs)
		}
		if string(beforeRecord.rawPayload) != string(afterRecord.rawPayload) {
			return fmt.Sprintf("peer_id %q raw payload before=%q after=%q", peerID, string(beforeRecord.rawPayload), string(afterRecord.rawPayload))
		}
	}
	return ""
}

func diffLastSeen(before map[string]LastSeenPeer, after map[string]LastSeenPeer) string {
	if len(before) != len(after) {
		return fmt.Sprintf("before=%d entries after=%d entries", len(before), len(after))
	}
	for peerID, beforeItem := range before {
		afterItem, ok := after[peerID]
		if !ok {
			return fmt.Sprintf("missing peer_id %q after mutation", peerID)
		}
		if beforeItem != afterItem {
			return fmt.Sprintf("peer_id %q before=%+v after=%+v", peerID, beforeItem, afterItem)
		}
	}
	return ""
}
