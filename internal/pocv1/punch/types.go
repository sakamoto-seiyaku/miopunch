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

package punch

import (
	"context"
	"crypto/ed25519"
	"net"
	"time"

	"github.com/miopunch/miopunch/internal/udpowner"
	legacywire "github.com/miopunch/miopunch/internal/wire"

	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/presence"
)

const (
	defaultAttemptConcurrency = 4
	defaultAttemptBudget      = 10 * time.Second
	defaultInnerTTL           = 30 * time.Second
	defaultPunchReadTimeout   = 2 * time.Second
)

// CandidateKind is the fixed current v1 UDP candidate kind set.
type CandidateKind string

const (
	CandidateKindHost  CandidateKind = "host"
	CandidateKindSrflx CandidateKind = "srflx"
)

// Candidate is one fixed current v1 UDP candidate.
//
// Local candidates are declared by the caller for the shared UDP owner/socket
// passed in Config. They describe the matrix and evidence surface but do not
// cause this package to bind additional local sockets per attempt.
type Candidate struct {
	Kind CandidateKind
	Addr string
}

// DialOffer is the fixed current v1 dial_offer body.
type DialOffer struct {
	DialID           string
	PunchToken       []byte
	Candidates       []Candidate
	MemberCredential []byte
}

// DialAnswer is the fixed current v1 dial_answer body.
type DialAnswer struct {
	DialID           string
	PunchToken       []byte
	Candidates       []Candidate
	MemberCredential []byte
}

// TrustedRemoteIdentity is the validated remote identity handoff folded into
// PathResult.
type TrustedRemoteIdentity struct {
	PeerID           string
	MemberCredential []byte
}

// AttemptEvidence is one bounded candidate-pair attempt record.
type AttemptEvidence struct {
	LocalCandidate  Candidate
	RemoteCandidate Candidate
	Result          string
	Detail          string
}

// PunchEvidence is the explainable current v1 punch result payload.
type PunchEvidence struct {
	DialID            string
	AttemptedPairs    []AttemptEvidence
	SelectedLocal     Candidate
	SelectedRemote    Candidate
	SelectedRemoteUDP string
}

// PathResult is the only output of current v1 dial/punch.
type PathResult struct {
	Conn           *net.UDPConn
	RemoteAddr     *net.UDPAddr
	RemoteIdentity TrustedRemoteIdentity
	Evidence       PunchEvidence
}

// Close releases the owned UDP path.
func (r PathResult) Close() error {
	if r.Conn == nil {
		return nil
	}
	return r.Conn.Close()
}

// Target is one current v1 dial target.
type Target struct {
	PeerID string
}

// Config is the concrete current v1 punch runtime configuration.
type Config struct {
	NetworkID           string
	AuthorityEd25519Pub ed25519.PublicKey
	Store               *persist.Store
	Discover            presence.DiscoverProjection
	LocalCandidates     []Candidate
	UDPConn             *net.UDPConn
	AttemptConcurrency  int
	AttemptBudget       time.Duration
	NowUnixMs           func() uint64
	NewMsgID            func() (string, error)
	NewDialID           func() (string, error)
	NewPunchToken       func() ([]byte, error)
	OpenPeerMessage     peerMessageOpener
	AttemptPair         AttemptPairFunc
}

// LoadedConfig is the normalized runtime config with persisted state resolved.
type LoadedConfig struct {
	NetworkID           string
	AuthorityEd25519Pub ed25519.PublicKey
	Store               *persist.Store
	Discover            presence.DiscoverProjection
	LocalCandidates     []Candidate
	UDPConn             *net.UDPConn
	AttemptConcurrency  int
	AttemptBudget       time.Duration
	NowUnixMs           func() uint64
	NewMsgID            func() (string, error)
	NewDialID           func() (string, error)
	NewPunchToken       func() ([]byte, error)
	OpenPeerMessage     peerMessageOpener
	AttemptPair         AttemptPairFunc

	DeviceKeys        persist.DeviceKeys
	LocalPeerID       string
	LocalEd25519Priv  ed25519.PrivateKey
	LocalEd25519Pub   ed25519.PublicKey
	LocalX25519Priv   []byte
	LocalX25519Pub    []byte
	SelfCredential    []byte
	RuntimeBroker     persist.RuntimeBroker
	TopicScope        persist.TopicScope
	RosterSnapshot    persist.RosterSnapshot
	TrustedRosterByID map[string]persist.RosterEntry
}

// AttemptPairFunc is the narrow leaf seam for one UDP candidate-pair attempt.
type AttemptPairFunc func(
	ctx context.Context,
	demux *udpowner.TraversalDemux,
	plan pairPlan,
	key []byte,
) (*net.UDPAddr, error)

// SelectedAttempt is one successful attempt winner.
type SelectedAttempt struct {
	LocalCandidate  Candidate
	RemoteCandidate Candidate
	Conn            *net.UDPConn
	RemoteAddr      *net.UDPAddr
}

// pairPlan is one bounded candidate-pair runtime unit.
type pairPlan struct {
	index  int
	local  Candidate
	remote Candidate
	sid    string
	conn   *net.UDPConn
	resp   *legacywire.NatHoleResp
}
