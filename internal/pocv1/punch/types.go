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
	"errors"
	"io"
	"net"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/udpowner"
	legacywire "github.com/miopunch/miopunch/internal/wire"

	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/presence"
)

const (
	defaultAttemptConcurrency = 4
	defaultAttemptBudget      = 10 * time.Second
	defaultInnerTTL           = 30 * time.Second
	defaultDirectTimeout      = 800 * time.Millisecond
	defaultDirectSendCount    = 3
	defaultDirectSendInterval = 100 * time.Millisecond
	defaultPunchReadTimeout   = 2 * time.Second
)

const (
	// PathDirectIPv4 identifies a selected host-to-host UDP IPv4 path.
	PathDirectIPv4 = "direct_ipv4"
	// PathDirectIPv6 identifies a selected host-to-host UDP IPv6 path.
	PathDirectIPv6 = "direct_ipv6"
	// PathPunchingIPv4 identifies a selected UDP IPv4 punching path.
	PathPunchingIPv4 = "punching_ipv4"
)

// SelectedUDPOwnership identifies who owns the selected UDP socket.
type SelectedUDPOwnership string

const (
	// SelectedUDPOwnershipRuntime means the selected UDP path uses Runtime's owner.
	SelectedUDPOwnershipRuntime SelectedUDPOwnership = "runtime"
	// SelectedUDPOwnershipTemporary means the selected UDP socket is session-owned.
	SelectedUDPOwnershipTemporary SelectedUDPOwnership = "temporary"
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

// UDPSnapshot is the current v1 UDP-only decision material exchanged by
// dial_offer and dial_answer.
type UDPSnapshot struct {
	DirectAddrs   []string `json:"direct_addrs,omitempty"`
	MappedAddrs   []string `json:"mapped_addrs,omitempty"`
	AssistedAddrs []string `json:"assisted_addrs,omitempty"`
}

// UDPDecision carries the answer-side decision output shared by both peers.
type UDPDecision struct {
	LocalResponse  legacywire.NatHoleResp `json:"local_response,omitempty"`
	RemoteResponse legacywire.NatHoleResp `json:"remote_response,omitempty"`
	AnalysisKey    string                 `json:"analysis_key,omitempty"`
	AnalyzerKey    string                 `json:"analyzer_key,omitempty"`
	Mode           int                    `json:"decision_mode"`
	Index          int                    `json:"decision_index"`
}

// DialOffer is the fixed current v1 dial_offer body.
type DialOffer struct {
	DialID           string
	PunchToken       []byte
	Candidates       []Candidate
	UDPSnapshot      UDPSnapshot
	P2PNetwork       connectivity.P2PNetwork
	P2PIPFamily      connectivity.P2PIPFamily
	MemberCredential []byte
}

// DialAnswer is the fixed current v1 dial_answer body.
type DialAnswer struct {
	DialID           string
	PunchToken       []byte
	Candidates       []Candidate
	UDPSnapshot      UDPSnapshot
	Decision         UDPDecision
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
	Path            string
	Result          string
	Detail          string
}

// PunchEvidence is the explainable current v1 punch result payload.
type PunchEvidence struct {
	DialID            string
	AttemptedPairs    []AttemptEvidence
	SelectedPath      string
	SelectedLocal     Candidate
	SelectedRemote    Candidate
	SelectedRemoteUDP string
}

// PathResult is the only output of current v1 dial/punch.
type PathResult struct {
	Conn                  *net.UDPConn
	RemoteAddr            *net.UDPAddr
	AllowedRemoteUDPAddrs []string
	RemoteIdentity        TrustedRemoteIdentity
	Evidence              PunchEvidence
	UDPOwnership          SelectedUDPOwnership
	RuntimeKCPPacket      net.PacketConn
	TemporaryUDPCloser    io.Closer
}

// Ownership returns the selected UDP ownership kind.
func (r PathResult) Ownership() SelectedUDPOwnership {
	if r.UDPOwnership != "" {
		return r.UDPOwnership
	}
	if r.Conn != nil || r.RuntimeKCPPacket != nil {
		return SelectedUDPOwnershipRuntime
	}
	return ""
}

// Close releases resources owned by a failed PathResult handoff.
func (r PathResult) Close() error {
	if r.Ownership() != SelectedUDPOwnershipTemporary {
		return nil
	}
	var err error
	if r.TemporaryUDPCloser != nil {
		err = r.TemporaryUDPCloser.Close()
	} else if r.Conn != nil {
		err = r.Conn.Close()
	}
	if errors.Is(err, net.ErrClosed) {
		return nil
	}
	return err
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
	UDPOwner            *udpowner.KCPOwner
	UDP6Conn            *net.UDPConn
	UDP6Owner           *udpowner.KCPOwner
	P2PNetwork          connectivity.P2PNetwork
	P2PIPFamily         connectivity.P2PIPFamily
	StunServers         []string
	StunExplicit        bool
	StunTimeout         time.Duration
	AttemptConcurrency  int
	AttemptBudget       time.Duration
	NowUnixMs           func() uint64
	NewMsgID            func() (string, error)
	NewDialID           func() (string, error)
	NewPunchToken       func() ([]byte, error)
	OpenPeerMessage     peerMessageOpener
	GatherUDPSnapshot   UDPSnapshotGatherer
	AttemptUDP          UDPAttemptFunc
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
	UDPOwner            *udpowner.KCPOwner
	UDP6Conn            *net.UDPConn
	UDP6Owner           *udpowner.KCPOwner
	P2PNetwork          connectivity.P2PNetwork
	P2PIPFamily         connectivity.P2PIPFamily
	StunServers         []string
	StunExplicit        bool
	StunTimeout         time.Duration
	AttemptConcurrency  int
	AttemptBudget       time.Duration
	NowUnixMs           func() uint64
	NewMsgID            func() (string, error)
	NewDialID           func() (string, error)
	NewPunchToken       func() ([]byte, error)
	OpenPeerMessage     peerMessageOpener
	GatherUDPSnapshot   UDPSnapshotGatherer
	AttemptUDP          UDPAttemptFunc
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

// UDPSnapshotGatherer captures the local runtime UDP address snapshot for one
// dial session.
type UDPSnapshotGatherer func(ctx context.Context, cfg LoadedConfig, sid string) (UDPSnapshot, error)

// UDPAttemptFunc executes one decision-ready UDP traversal attempt.
type UDPAttemptFunc func(
	ctx context.Context,
	sid string,
	key []byte,
	udp4Conn *net.UDPConn,
	udp6Conn *net.UDPConn,
	resp *legacywire.NatHoleResp,
	udp4Demux *udpowner.TraversalDemux,
	udp6Demux *udpowner.TraversalDemux,
) (*connectivity.AttemptResult, error)

// AttemptPairFunc is the narrow leaf seam for one UDP candidate-pair attempt.
type AttemptPairFunc func(
	ctx context.Context,
	demux *udpowner.TraversalDemux,
	plan pairPlan,
	key []byte,
) (AttemptPairResult, error)

// AttemptPairResult reports one successful candidate-pair attempt result.
type AttemptPairResult struct {
	RemoteAddr *net.UDPAddr
	Path       string
	Detail     string
}

// SelectedAttempt is one successful attempt winner.
type SelectedAttempt struct {
	LocalCandidate  Candidate
	RemoteCandidate Candidate
	Conn            *net.UDPConn
	RemoteAddr      *net.UDPAddr
	Path            string
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
