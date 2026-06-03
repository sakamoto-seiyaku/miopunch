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
	"crypto/rand"
	"fmt"
	"net"
	"slices"
	"strings"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/pocv1/peere2e"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/presence"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
	signalmqtt "github.com/miopunch/miopunch/internal/signaling/mqtt"
)

func loadConfig(cfg Config) (LoadedConfig, error) {
	if cfg.Store == nil {
		return LoadedConfig{}, fmt.Errorf("%w: persistence store is required", ErrInvalidConfig)
	}
	if cfg.UDPConn == nil {
		return LoadedConfig{}, fmt.Errorf("%w: udp conn is required", ErrInvalidConfig)
	}
	networkID, err := wire.CanonicalizeNetworkID(cfg.NetworkID)
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("%w: canonicalize network_id: %w", ErrInvalidConfig, err)
	}
	if len(cfg.AuthorityEd25519Pub) != ed25519.PublicKeySize {
		return LoadedConfig{}, fmt.Errorf("%w: invalid authority ed25519 key length: %d", ErrInvalidConfig, len(cfg.AuthorityEd25519Pub))
	}
	attemptConcurrency := cfg.AttemptConcurrency
	if attemptConcurrency <= 0 {
		attemptConcurrency = defaultAttemptConcurrency
	}
	attemptBudget := cfg.AttemptBudget
	if attemptBudget <= 0 {
		attemptBudget = defaultAttemptBudget
	}
	stunTimeout := cfg.StunTimeout
	if stunTimeout <= 0 {
		stunTimeout = 3 * time.Second
	}
	p2pNetwork, err := normalizePOCV1P2PNetwork(cfg.P2PNetwork)
	if err != nil {
		return LoadedConfig{}, err
	}
	p2pIPFamily, err := normalizePOCV1P2PIPFamily(cfg.P2PIPFamily, cfg.UDP6Conn)
	if err != nil {
		return LoadedConfig{}, err
	}

	nowUnixMs := cfg.NowUnixMs
	if nowUnixMs == nil {
		nowUnixMs = func() uint64 { return uint64(time.Now().UnixMilli()) }
	}
	newMsgID := cfg.NewMsgID
	if newMsgID == nil {
		newMsgID = wire.NewMsgID
	}
	newDialID := cfg.NewDialID
	if newDialID == nil {
		newDialID = wire.NewMsgID
	}
	newPunchToken := cfg.NewPunchToken
	if newPunchToken == nil {
		newPunchToken = randomPunchToken
	}
	openPeerMessage := cfg.OpenPeerMessage
	if openPeerMessage == nil {
		openPeerMessage = defaultPeerMessageOpener
	}
	gatherUDPSnapshot := cfg.GatherUDPSnapshot
	if gatherUDPSnapshot == nil {
		gatherUDPSnapshot = gatherUDPSnapshotDefault
	}
	attemptUDP := cfg.AttemptUDP
	attemptPair := cfg.AttemptPair
	if attemptPair == nil {
		attemptPair = defaultAttemptPair
	}

	deviceKeys, err := cfg.Store.LoadDeviceKeys()
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("load device keys: %w", err)
	}
	localPeerID, err := deviceKeys.PeerID()
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("derive local peer_id: %w", err)
	}
	localEd25519Priv, err := deviceKeys.Ed25519PrivateKey()
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("derive local ed25519 private key: %w", err)
	}
	localEd25519Pub, err := deviceKeys.Ed25519PublicKey()
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("derive local ed25519 public key: %w", err)
	}
	localX25519Pub, err := deviceKeys.X25519PublicKey()
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("derive local x25519 public key: %w", err)
	}
	selfCredential, err := cfg.Store.LoadSelfMemberCredential(networkID)
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("load self member credential: %w", err)
	}
	runtimeBroker, err := cfg.Store.LoadRuntimeBroker(networkID)
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("load runtime broker: %w", err)
	}
	topicScope, err := cfg.Store.LoadTopicScope(networkID)
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("load topic scope: %w", err)
	}
	rosterSnapshot, err := cfg.Store.LoadRosterSnapshot(networkID)
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("load roster snapshot: %w", err)
	}
	trustedRosterByID := make(map[string]persist.RosterEntry, len(rosterSnapshot.Entries))
	for _, entry := range rosterSnapshot.Entries {
		trustedRosterByID[entry.PeerID] = entry
	}

	localCandidates, err := normalizeOptionalCandidatesForIPFamily(cfg.LocalCandidates, p2pIPFamily)
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("%w: %w", ErrInvalidConfig, err)
	}

	return LoadedConfig{
		NetworkID:           networkID,
		AuthorityEd25519Pub: append(ed25519.PublicKey(nil), cfg.AuthorityEd25519Pub...),
		Store:               cfg.Store,
		Discover:            cfg.Discover,
		LocalCandidates:     localCandidates,
		UDPConn:             cfg.UDPConn,
		UDPOwner:            cfg.UDPOwner,
		UDP6Conn:            cfg.UDP6Conn,
		UDP6Owner:           cfg.UDP6Owner,
		P2PNetwork:          p2pNetwork,
		P2PIPFamily:         p2pIPFamily,
		StunServers:         append([]string(nil), cfg.StunServers...),
		StunExplicit:        cfg.StunExplicit,
		StunTimeout:         stunTimeout,
		AttemptConcurrency:  attemptConcurrency,
		AttemptBudget:       attemptBudget,
		NowUnixMs:           nowUnixMs,
		NewMsgID:            newMsgID,
		NewDialID:           newDialID,
		NewPunchToken:       newPunchToken,
		OpenPeerMessage:     openPeerMessage,
		GatherUDPSnapshot:   gatherUDPSnapshot,
		AttemptUDP:          attemptUDP,
		AttemptPair:         attemptPair,
		DeviceKeys:          deviceKeys,
		LocalPeerID:         localPeerID,
		LocalEd25519Priv:    localEd25519Priv,
		LocalEd25519Pub:     localEd25519Pub,
		LocalX25519Priv:     append([]byte(nil), deviceKeys.X25519PrivateKey...),
		LocalX25519Pub:      localX25519Pub,
		SelfCredential:      append([]byte(nil), selfCredential...),
		RuntimeBroker:       runtimeBroker,
		TopicScope:          topicScope,
		RosterSnapshot:      rosterSnapshot,
		TrustedRosterByID:   trustedRosterByID,
	}, nil
}

func normalizePOCV1P2PNetwork(value connectivity.P2PNetwork) (connectivity.P2PNetwork, error) {
	network, err := connectivity.ParseP2PNetwork(string(value))
	if err != nil {
		return "", fmt.Errorf("%w: %w", ErrInvalidConfig, err)
	}
	switch network {
	case connectivity.P2PNetworkAuto, connectivity.P2PNetworkUDPOnly:
		return connectivity.P2PNetworkUDPOnly, nil
	case connectivity.P2PNetworkTCPOnly:
		return "", fmt.Errorf("%w: tcp_only is not supported by current POC v1", ErrUnsupportedP2PNetwork)
	default:
		return "", fmt.Errorf("%w: invalid p2p network %q", ErrInvalidConfig, network)
	}
}

func normalizePOCV1P2PIPFamily(value connectivity.P2PIPFamily, udp6Conn *net.UDPConn) (connectivity.P2PIPFamily, error) {
	family, err := connectivity.ParseP2PIPFamily(string(value))
	if err != nil {
		return "", fmt.Errorf("%w: %w", ErrInvalidConfig, err)
	}
	if family == connectivity.P2PIPFamilyAuto && udp6Conn == nil {
		return connectivity.P2PIPFamilyV4, nil
	}
	return family, nil
}

func randomPunchToken() ([]byte, error) {
	token := make([]byte, 16)
	if _, err := rand.Read(token); err != nil {
		return nil, err
	}
	return token, nil
}

func normalizeCandidates(in []Candidate) ([]Candidate, error) {
	out := make([]Candidate, 0, len(in))
	seen := make(map[string]struct{}, len(in))
	for i, candidate := range in {
		addr := strings.TrimSpace(candidate.Addr)
		if addr == "" {
			return nil, fmt.Errorf("candidate %d empty addr", i)
		}
		if _, err := net.ResolveUDPAddr("udp", addr); err != nil {
			return nil, fmt.Errorf("candidate %d resolve addr %q: %w", i, addr, err)
		}
		switch candidate.Kind {
		case CandidateKindHost, CandidateKindSrflx:
		default:
			return nil, fmt.Errorf("candidate %d invalid kind %q", i, candidate.Kind)
		}
		key := string(candidate.Kind) + "|" + addr
		if _, ok := seen[key]; ok {
			continue
		}
		seen[key] = struct{}{}
		out = append(out, Candidate{
			Kind: candidate.Kind,
			Addr: addr,
		})
	}
	if len(out) == 0 {
		return nil, fmt.Errorf("no candidates")
	}
	slices.SortFunc(out, func(a, b Candidate) int {
		if a.Kind != b.Kind {
			return strings.Compare(string(a.Kind), string(b.Kind))
		}
		return strings.Compare(a.Addr, b.Addr)
	})
	return out, nil
}

func normalizeOptionalCandidates(in []Candidate) ([]Candidate, error) {
	if len(in) == 0 {
		return nil, nil
	}
	return normalizeCandidates(in)
}

func normalizeCandidatesForIPFamily(in []Candidate, family connectivity.P2PIPFamily) ([]Candidate, error) {
	candidates, err := normalizeCandidates(in)
	if err != nil {
		return nil, err
	}
	if family == connectivity.P2PIPFamilyAuto {
		return candidates, nil
	}
	out := make([]Candidate, 0, len(candidates))
	for _, candidate := range candidates {
		host, _, err := net.SplitHostPort(candidate.Addr)
		if err != nil {
			return nil, err
		}
		ip := net.ParseIP(host)
		if ip == nil {
			return nil, fmt.Errorf("candidate addr %q has invalid ip", candidate.Addr)
		}
		switch family {
		case connectivity.P2PIPFamilyV4:
			if ip.To4() != nil {
				out = append(out, candidate)
			}
		case connectivity.P2PIPFamilyV6:
			if ip.To4() == nil {
				out = append(out, candidate)
			}
		default:
			return nil, fmt.Errorf("invalid p2p ip family %q", family)
		}
	}
	if len(out) == 0 {
		return nil, fmt.Errorf("no candidates for p2p ip family %s", family)
	}
	return out, nil
}

func normalizeOptionalCandidatesForIPFamily(in []Candidate, family connectivity.P2PIPFamily) ([]Candidate, error) {
	if len(in) == 0 {
		return nil, nil
	}
	return normalizeCandidatesForIPFamily(in, family)
}

func targetOnline(discover presence.DiscoverProjection, peerID string) bool {
	for _, peer := range discover.Peers {
		if peer.PeerID == peerID && peer.OnlineState == presence.OnlineStateOnline {
			return true
		}
	}
	return false
}

func withAttemptBudget(ctx context.Context, budget time.Duration) (context.Context, context.CancelFunc) {
	budgetCtx, budgetCancel := context.WithCancelCause(ctx)
	timer := time.AfterFunc(budget, func() {
		budgetCancel(ErrAttemptBudgetExceeded)
	})
	return budgetCtx, func() {
		timer.Stop()
		budgetCancel(context.Canceled)
	}
}

type peerMessageOpener func(ctx context.Context, cfg LoadedConfig) (peerMessageSession, error)

type peerMessageSession interface {
	Close() error
	PublishInner(
		ctx context.Context,
		topic string,
		inner pocwire.InnerMessage,
		recipientX25519PublicKey []byte,
		opts peere2e.SealOptions,
	) (pocwire.OuterHeader, error)
	WaitOpened(
		ctx context.Context,
		recipientX25519PrivateKey []byte,
		opts peere2e.OpenOptions,
	) (signalmqtt.OpenedPeerMessage, error)
}

func defaultPeerMessageOpener(ctx context.Context, cfg LoadedConfig) (peerMessageSession, error) {
	inbox, err := cfg.TopicScope.InboxTopic(cfg.LocalPeerID)
	if err != nil {
		return nil, fmt.Errorf("derive local inbox topic: %w", err)
	}
	return signalmqtt.OpenPeerMessageSession(ctx, signalmqtt.PeerMessageConfig{
		BrokerURL:       cfg.RuntimeBroker.Endpoint,
		SubscribeTopics: []string{inbox},
	})
}
