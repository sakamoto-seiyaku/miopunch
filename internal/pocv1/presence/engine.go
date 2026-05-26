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
	"time"

	"github.com/miopunch/miopunch/internal/pocv1/persist"
)

type observationRecord struct {
	obs        Observation
	rawPayload []byte
}

// Engine owns the in-memory presence observation set and discover merge logic.
type Engine struct {
	cfg          Config
	rosterByPeer map[string]persist.RosterEntry
	rosterOrder  []persist.RosterEntry
	observed     map[string]observationRecord
	diagnostics  []Diagnostic
	lastSeen     map[string]LastSeenPeer
}

// NewEngine creates one current v1 presence merge engine.
func NewEngine(cfg Config) (*Engine, error) {
	normalizedCfg, err := normalizeConfig(cfg)
	if err != nil {
		return nil, err
	}

	rosterByPeer := make(map[string]persist.RosterEntry, len(normalizedCfg.RosterSnapshot.Entries))
	rosterOrder := make([]persist.RosterEntry, 0, len(normalizedCfg.RosterSnapshot.Entries))
	for _, entry := range normalizedCfg.RosterSnapshot.Entries {
		rosterByPeer[entry.PeerID] = persist.RosterEntry{
			PeerID:           entry.PeerID,
			MemberCredential: append([]byte(nil), entry.MemberCredential...),
			DeviceName:       entry.DeviceName,
			Platform:         entry.Platform,
		}
		rosterOrder = append(rosterOrder, persist.RosterEntry{
			PeerID:           entry.PeerID,
			MemberCredential: append([]byte(nil), entry.MemberCredential...),
			DeviceName:       entry.DeviceName,
			Platform:         entry.Platform,
		})
	}

	return &Engine{
		cfg:          normalizedCfg,
		rosterByPeer: rosterByPeer,
		rosterOrder:  rosterOrder,
		observed:     make(map[string]observationRecord),
		diagnostics:  make([]Diagnostic, 0),
		lastSeen:     make(map[string]LastSeenPeer),
	}, nil
}

// ApplyMessage parses and merges one MQTT topic/payload observation.
//
// It returns true when the engine state changed.
func (e *Engine) ApplyMessage(topic string, rawPayload []byte) bool {
	obs, diag := ParseObservation(topic, rawPayload, e.cfg.TopicScope)
	if diag != nil {
		e.diagnostics = append(e.diagnostics, *diag)
		return true
	}
	return e.ApplyObservation(obs, rawPayload)
}

// ApplyObservation merges one already-normalized observation.
//
// It returns true when the engine state changed.
func (e *Engine) ApplyObservation(obs Observation, rawPayload []byte) bool {
	prev, ok := e.observed[obs.PeerID]
	if ok && prev.obs.OnlineState == obs.OnlineState && bytes.Equal(prev.rawPayload, rawPayload) {
		return false
	}

	e.observed[obs.PeerID] = observationRecord{
		obs:        obs,
		rawPayload: append([]byte(nil), rawPayload...),
	}
	e.lastSeen[obs.PeerID] = MergeLastSeen(e.lastSeen[obs.PeerID], obs)

	if obs.PeerID != e.cfg.SelfPeerID {
		if _, ok := e.rosterByPeer[obs.PeerID]; !ok {
			e.diagnostics = append(e.diagnostics, Diagnostic{
				Kind:    DiagnosticUnknownPeer,
				PeerID:  obs.PeerID,
				Message: "presence observation is outside the trusted roster",
			})
		}
	}

	return true
}

// View materializes the current roster-bounded discover view.
func (e *Engine) View(now time.Time) DiscoverView {
	observedAtUnixMs := uint64(now.UTC().UnixMilli())
	peers := make([]DiscoverPeer, 0, len(e.rosterOrder))
	for _, rosterEntry := range e.rosterOrder {
		if rosterEntry.PeerID == e.cfg.SelfPeerID {
			continue
		}

		peer := DiscoverPeer{
			PeerID:      rosterEntry.PeerID,
			OnlineState: OnlineStateOffline,
			DeviceName:  rosterEntry.DeviceName,
			Platform:    rosterEntry.Platform,
		}

		if record, ok := e.observed[rosterEntry.PeerID]; ok {
			peer.OnlineState = record.obs.OnlineState
			if peer.DeviceName == "" {
				peer.DeviceName = record.obs.DeviceName
			}
			if peer.Platform == "" {
				peer.Platform = record.obs.Platform
			}
			peer.AppVer = record.obs.AppVer
			peer.LastObservedUnixMs = record.obs.LastObservedUnixMs
		}

		peers = append(peers, peer)
	}

	return DiscoverView{
		NetworkID:        e.cfg.NetworkID,
		SelfPeerID:       e.cfg.SelfPeerID,
		ObservedAtUnixMs: observedAtUnixMs,
		Peers:            peers,
	}
}

// Diagnostics returns a copy of the accumulated presence diagnostics.
func (e *Engine) Diagnostics() []Diagnostic {
	out := make([]Diagnostic, len(e.diagnostics))
	copy(out, e.diagnostics)
	return out
}

// LastSeen returns a copy of the in-memory last-seen map.
func (e *Engine) LastSeen() []LastSeenPeer {
	out := make([]LastSeenPeer, 0, len(e.lastSeen))
	for _, item := range e.lastSeen {
		out = append(out, item)
	}
	return out
}

// DialOnlineSurface returns the `04` dial/punch handoff derived only from
// DiscoverPeer online state.
func DialOnlineSurface(view DiscoverView) map[string]OnlineState {
	out := make(map[string]OnlineState, len(view.Peers))
	for _, peer := range view.Peers {
		out[peer.PeerID] = peer.OnlineState
	}
	return out
}

// ProjectView returns the `07` discover-only handoff for downstream runtime
// consumers.
//
// ProjectView deep-copies the peer slice and exposes only discover-owned
// fields.
func ProjectView(view DiscoverView) DiscoverProjection {
	peers := make([]DiscoverProjectionPeer, 0, len(view.Peers))
	for _, peer := range view.Peers {
		peers = append(peers, DiscoverProjectionPeer{
			PeerID:             peer.PeerID,
			OnlineState:        peer.OnlineState,
			DeviceName:         peer.DeviceName,
			Platform:           peer.Platform,
			AppVer:             peer.AppVer,
			LastObservedUnixMs: peer.LastObservedUnixMs,
		})
	}

	return DiscoverProjection{
		NetworkID:        view.NetworkID,
		SelfPeerID:       view.SelfPeerID,
		ObservedAtUnixMs: view.ObservedAtUnixMs,
		Peers:            peers,
	}
}

// MergeLastSeen applies one observation to the minimal last-seen object model.
func MergeLastSeen(prior LastSeenPeer, obs Observation) LastSeenPeer {
	next := LastSeenPeer{
		PeerID:             obs.PeerID,
		LastState:          obs.OnlineState,
		LastObservedUnixMs: obs.LastObservedUnixMs,
		LastOnlineUnixMs:   prior.LastOnlineUnixMs,
	}
	if obs.OnlineState == OnlineStateOnline {
		next.LastOnlineUnixMs = obs.LastObservedUnixMs
	}
	return next
}
