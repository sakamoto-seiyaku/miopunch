package task

import (
	"sort"
	"strings"

	"github.com/miopunch/miopunch/internal/pocstate"
)

func (m *Manager) ensureLocalSeedPeer(selfID pocstate.Identity) (seedPeerV0, error) {
	st, err := m.loadState()
	if err != nil {
		return seedPeerV0{}, err
	}
	if st.Local == nil {
		st.Local = &pocstate.LocalConfig{}
	}
	st.Local.NormalizeDefaults()
	st.Local.PeerID = selfID.PeerID
	if strings.TrimSpace(st.Local.ProxyName) == "" {
		st.Local.ProxyName = selfID.PeerID
	}
	if strings.TrimSpace(st.Local.SecretKey) == "" {
		secretKey, _, err := newSecretKeyB64URLNoPad()
		if err != nil {
			return seedPeerV0{}, err
		}
		st.Local.SecretKey = secretKey
	}
	st.EnsureLocalDefaults()

	if err := m.saveState(st); err != nil {
		return seedPeerV0{}, err
	}
	return seedPeerFromLocalConfig(selfID.PeerID, *st.Local), nil
}

func seedPeerFromLocalConfig(peerID string, cfg pocstate.LocalConfig) seedPeerV0 {
	return seedPeerV0{
		PeerID:      strings.TrimSpace(peerID),
		ProxyName:   strings.TrimSpace(cfg.ProxyName),
		SecretKey:   strings.TrimSpace(cfg.SecretKey),
		MQTTBroker:  strings.TrimSpace(cfg.MQTTBroker),
		TopicPrefix: strings.TrimSpace(cfg.TopicPrefix),
		V4Hint:      pocstate.NormalizeV4Hint(cfg.V4Hint),
		V6Hint:      pocstate.NormalizeV6Hint(cfg.V6Hint),
		DataProto:   strings.TrimSpace(cfg.DataProto),
		QUICCC:      strings.TrimSpace(cfg.QUICCC),
	}
}

func seedPeerFromPeerConfig(peerID string, cfg pocstate.PeerConfig) (seedPeerV0, bool) {
	cfg.NormalizeDefaults()
	sp := seedPeerV0{
		PeerID:      strings.TrimSpace(peerID),
		ProxyName:   strings.TrimSpace(cfg.ProxyName),
		SecretKey:   strings.TrimSpace(cfg.SecretKey),
		MQTTBroker:  strings.TrimSpace(cfg.MQTTBroker),
		TopicPrefix: strings.TrimSpace(cfg.TopicPrefix),
		V4Hint:      pocstate.NormalizeV4Hint(cfg.V4Hint),
		V6Hint:      pocstate.NormalizeV6Hint(cfg.V6Hint),
		DataProto:   strings.TrimSpace(cfg.DataProto),
		QUICCC:      strings.TrimSpace(cfg.QUICCC),
	}
	return sp, sp.validForSeed()
}

func (sp seedPeerV0) validForSeed() bool {
	return strings.TrimSpace(sp.PeerID) != "" &&
		strings.TrimSpace(sp.ProxyName) != "" &&
		strings.TrimSpace(sp.SecretKey) != "" &&
		strings.TrimSpace(sp.MQTTBroker) != "" &&
		strings.TrimSpace(sp.TopicPrefix) != ""
}

func (sp seedPeerV0) peerConfig() (pocstate.PeerConfig, bool) {
	if !sp.validForSeed() {
		return pocstate.PeerConfig{}, false
	}
	return pocstate.PeerConfig{
		ProxyName:   strings.TrimSpace(sp.ProxyName),
		SecretKey:   strings.TrimSpace(sp.SecretKey),
		MQTTBroker:  strings.TrimSpace(sp.MQTTBroker),
		TopicPrefix: strings.TrimSpace(sp.TopicPrefix),
		V4Hint:      pocstate.NormalizeV4Hint(sp.V4Hint),
		V6Hint:      pocstate.NormalizeV6Hint(sp.V6Hint),
		DataProto:   strings.TrimSpace(sp.DataProto),
		QUICCC:      strings.TrimSpace(sp.QUICCC),
	}, true
}

func (sp *seedPeerV0) v4Hint() string {
	if sp == nil {
		return ""
	}
	return pocstate.NormalizeV4Hint(sp.V4Hint)
}

func (sp *seedPeerV0) v6Hint() string {
	if sp == nil {
		return ""
	}
	return pocstate.NormalizeV6Hint(sp.V6Hint)
}

func bootstrapRecommendations(selfPeerID string, joinerPeerID string, st pocstate.State) []pocstate.BootstrapPeerEvidenceV0 {
	selfPeerID = strings.TrimSpace(selfPeerID)
	joinerPeerID = strings.TrimSpace(joinerPeerID)

	out := make([]pocstate.BootstrapPeerEvidenceV0, 0, 2)
	if selfPeerID != "" && selfPeerID != joinerPeerID {
		out = append(out, pocstate.BootstrapPeerEvidenceV0{
			PeerID: selfPeerID,
			Bucket: "direct",
			Reason: "approver_admin",
		})
	}

	candidates := sortedBootstrapCandidates(st.Peers)
	for _, candidate := range candidates {
		if len(out) >= 2 {
			break
		}
		peerID := candidate.peerID
		if peerID == "" || peerID == selfPeerID || peerID == joinerPeerID {
			continue
		}
		if _, ok := seedPeerFromPeerConfig(peerID, st.Peers[peerID]); !ok {
			continue
		}
		out = append(out, pocstate.BootstrapPeerEvidenceV0{
			PeerID: peerID,
			Bucket: candidate.bucket,
			Reason: "known_joined_seed",
		})
	}
	return out
}

func seedPeersForRecommendations(recs []pocstate.BootstrapPeerEvidenceV0, selfPeerID string, st pocstate.State) []seedPeerV0 {
	out := make([]seedPeerV0, 0, len(recs))
	for _, rec := range recs {
		peerID := strings.TrimSpace(rec.PeerID)
		if peerID == "" {
			continue
		}
		if peerID == strings.TrimSpace(selfPeerID) {
			if st.Local == nil {
				continue
			}
			sp := seedPeerFromLocalConfig(peerID, *st.Local)
			if sp.validForSeed() {
				out = append(out, sp)
			}
			continue
		}
		sp, ok := seedPeerFromPeerConfig(peerID, st.Peers[peerID])
		if ok {
			out = append(out, sp)
		}
	}
	return out
}

func sortedPeerIDs(peers map[string]pocstate.PeerConfig) []string {
	ids := make([]string, 0, len(peers))
	for peerID := range peers {
		peerID = strings.TrimSpace(peerID)
		if peerID != "" {
			ids = append(ids, peerID)
		}
	}
	sort.Strings(ids)
	return ids
}

type bootstrapCandidate struct {
	peerID string
	bucket string
	v4Rank int
	v6Rank int
}

func sortedBootstrapCandidates(peers map[string]pocstate.PeerConfig) []bootstrapCandidate {
	candidates := make([]bootstrapCandidate, 0, len(peers))
	for _, peerID := range sortedPeerIDs(peers) {
		cfg := peers[peerID]
		cfg.NormalizeDefaults()
		bucket := pocstate.ReachabilityBucket(cfg.V4Hint, cfg.V6Hint)
		candidates = append(candidates, bootstrapCandidate{
			peerID: peerID,
			bucket: bucket,
			v4Rank: pocstate.ReachabilityRank(cfg.V4Hint),
			v6Rank: pocstate.ReachabilityRank(cfg.V6Hint),
		})
	}
	sort.SliceStable(candidates, func(i, j int) bool {
		left := pocstate.ReachabilityRank(candidates[i].bucket)
		right := pocstate.ReachabilityRank(candidates[j].bucket)
		if left != right {
			return left < right
		}
		if candidates[i].v6Rank != candidates[j].v6Rank {
			return candidates[i].v6Rank < candidates[j].v6Rank
		}
		if candidates[i].v4Rank != candidates[j].v4Rank {
			return candidates[i].v4Rank < candidates[j].v4Rank
		}
		return candidates[i].peerID < candidates[j].peerID
	})
	return candidates
}
