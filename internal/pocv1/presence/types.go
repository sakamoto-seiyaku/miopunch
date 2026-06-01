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

// PayloadVersion is the fixed current v1 presence payload version.
const PayloadVersion = 1

// OnlineState is the current v1 presence online/offline state.
type OnlineState string

const (
	// OnlineStateOnline marks a peer as currently online.
	OnlineStateOnline OnlineState = "online"
	// OnlineStateOffline marks a peer as currently offline.
	OnlineStateOffline OnlineState = "offline"
)

// IsValid reports whether state is one of the fixed current v1 values.
func (s OnlineState) IsValid() bool {
	return s == OnlineStateOnline || s == OnlineStateOffline
}

// Observation is one normalized current v1 presence observation.
type Observation struct {
	PeerID             string
	OnlineState        OnlineState
	DeviceName         string
	Platform           string
	AppVer             string
	LastObservedUnixMs uint64
}

// DiscoverView is the current v1 discover-owned output contract.
type DiscoverView struct {
	NetworkID        string
	SelfPeerID       string
	ObservedAtUnixMs uint64
	Peers            []DiscoverPeer
}

// DiscoverPeer is one trusted remote peer in a current v1 discover view.
type DiscoverPeer struct {
	PeerID             string
	OnlineState        OnlineState
	DeviceName         string
	Platform           string
	AppVer             string
	LastObservedUnixMs uint64
}

// DiscoverProjection is the discover-only handoff shape for downstream runtime
// consumers.
type DiscoverProjection struct {
	NetworkID        string
	SelfPeerID       string
	ObservedAtUnixMs uint64
	Peers            []DiscoverProjectionPeer
}

// DiscoverProjectionPeer is one projected discover peer handed to downstream
// runtime consumers.
type DiscoverProjectionPeer struct {
	PeerID             string
	OnlineState        OnlineState
	DeviceName         string
	Platform           string
	AppVer             string
	LastObservedUnixMs uint64
}

// LastSeenPeer is the minimal current v1 last-seen presence model.
type LastSeenPeer struct {
	PeerID             string
	LastState          OnlineState
	LastObservedUnixMs uint64
	LastOnlineUnixMs   uint64
}

// DiagnosticKind identifies a presence observation rejection or boundary event.
type DiagnosticKind string

const (
	// DiagnosticMalformedJSON reports a payload that could not be decoded.
	DiagnosticMalformedJSON DiagnosticKind = "malformed_json"
	// DiagnosticUnsupportedVersion reports an unsupported payload version.
	DiagnosticUnsupportedVersion DiagnosticKind = "unsupported_version"
	// DiagnosticInvalidPeerID reports an invalid payload peer_id.
	DiagnosticInvalidPeerID DiagnosticKind = "invalid_peer_id"
	// DiagnosticInvalidState reports an unsupported payload state value.
	DiagnosticInvalidState DiagnosticKind = "invalid_state"
	// DiagnosticTopicMismatch reports a topic/path payload mismatch.
	DiagnosticTopicMismatch DiagnosticKind = "topic_mismatch"
	// DiagnosticUnknownPeer reports a valid observation outside the trusted roster.
	DiagnosticUnknownPeer DiagnosticKind = "unknown_peer"
)

// Diagnostic is one typed presence diagnostic record.
type Diagnostic struct {
	Kind    DiagnosticKind
	Topic   string
	PeerID  string
	Message string
}
