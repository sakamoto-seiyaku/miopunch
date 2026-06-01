package task

import (
	"hash/fnv"
	"sort"
	"strings"

	"github.com/miopunch/miopunch/internal/pocstate"
)

type topologyNeighborCandidate struct {
	member   TopologyMember
	bucket   string
	dialable bool
}

func selectTopologyNeighbors(
	selfPeerID string,
	members []TopologyMember,
	bootstrap TopologyBootstrap,
	st pocstate.State,
	targetK int,
) []TopologyNeighborSelection {
	selfPeerID = strings.TrimSpace(selfPeerID)
	if selfPeerID == "" || targetK <= 0 {
		return []TopologyNeighborSelection{}
	}

	byBucket := make(map[string][]topologyNeighborCandidate)
	seen := make(map[string]struct{}, len(members))
	for _, mem := range members {
		peerID := strings.TrimSpace(mem.PeerID)
		if peerID == "" || peerID == selfPeerID || mem.Revoked {
			continue
		}
		if _, ok := seen[peerID]; ok {
			continue
		}
		seen[peerID] = struct{}{}
		bucket := pocstate.ReachabilityBucket(mem.V4Hint, mem.V6Hint)
		_, dialable := seedPeerFromPeerConfig(peerID, st.Peers[peerID])
		byBucket[bucket] = append(byBucket[bucket], topologyNeighborCandidate{
			member:   mem,
			bucket:   bucket,
			dialable: dialable,
		})
	}

	for _, candidate := range bootstrapNeighborCandidates(selfPeerID, bootstrap, st, seen) {
		byBucket[candidate.bucket] = append(byBucket[candidate.bucket], candidate)
	}

	for bucket := range byBucket {
		candidates := byBucket[bucket]
		sort.Slice(candidates, func(i, j int) bool {
			if candidates[i].member.Role != candidates[j].member.Role {
				if candidates[i].member.Role == "admin" || candidates[i].member.Role == "owner" {
					return false
				}
				if candidates[j].member.Role == "admin" || candidates[j].member.Role == "owner" {
					return true
				}
			}
			return candidates[i].member.PeerID < candidates[j].member.PeerID
		})
		byBucket[bucket] = rotateTopologyCandidates(candidates, topologyRotationOffset(selfPeerID, bucket, len(candidates)))
	}

	out := make([]TopologyNeighborSelection, 0, targetK)
	out = appendTopologyNeighborSelections(out, byBucket, targetK, true)
	out = appendTopologyNeighborSelections(out, byBucket, targetK, false)
	return out
}

func bootstrapNeighborCandidates(
	selfPeerID string,
	bootstrap TopologyBootstrap,
	st pocstate.State,
	seen map[string]struct{},
) []topologyNeighborCandidate {
	out := make([]topologyNeighborCandidate, 0)
	for _, evidence := range append(bootstrap.Recommendations, bootstrap.MoreRounds...) {
		peerID := strings.TrimSpace(evidence.PeerID)
		if peerID == "" || peerID == selfPeerID {
			continue
		}
		if _, ok := seen[peerID]; ok {
			continue
		}
		cfg, ok := st.Peers[peerID]
		if !ok {
			continue
		}
		if _, ok := seedPeerFromPeerConfig(peerID, cfg); !ok {
			continue
		}
		cfg.NormalizeDefaults()
		bucket := strings.TrimSpace(evidence.Bucket)
		if bucket == "" {
			bucket = pocstate.ReachabilityBucket(cfg.V4Hint, cfg.V6Hint)
		}
		seen[peerID] = struct{}{}
		out = append(out, topologyNeighborCandidate{
			member: TopologyMember{
				PeerID: peerID,
				Role:   topologyBootstrapRole(evidence),
				V4Hint: cfg.V4Hint,
				V6Hint: cfg.V6Hint,
			},
			bucket:   bucket,
			dialable: true,
		})
	}
	return out
}

func topologyBootstrapRole(evidence TopologyPeerEvidence) string {
	switch strings.TrimSpace(evidence.Reason) {
	case "approver_admin":
		return "admin"
	case "known_joined_seed", "bootstrap_more_response":
		return "member"
	default:
		return "unknown"
	}
}

func appendTopologyNeighborSelections(
	out []TopologyNeighborSelection,
	byBucket map[string][]topologyNeighborCandidate,
	targetK int,
	dialable bool,
) []TopologyNeighborSelection {
	for _, bucket := range topologyBucketOrder() {
		for _, candidate := range byBucket[bucket] {
			if len(out) >= targetK {
				return out
			}
			if candidate.dialable != dialable {
				continue
			}
			out = append(out, TopologyNeighborSelection{
				PeerID:   candidate.member.PeerID,
				Bucket:   candidate.bucket,
				Role:     candidate.member.Role,
				Reason:   "reachability_bucket_rotation",
				Dialable: candidate.dialable,
			})
		}
	}
	return out
}

func topologyBucketOrder() []string {
	return []string{
		pocstate.ReachabilityHintDirect,
		pocstate.ReachabilityHintEasy,
		pocstate.ReachabilityHintHard1,
		pocstate.ReachabilityHintHard2,
		pocstate.ReachabilityHintUnknown,
		pocstate.ReachabilityHintNone,
	}
}

func rotateTopologyCandidates(candidates []topologyNeighborCandidate, offset int) []topologyNeighborCandidate {
	if len(candidates) == 0 || offset == 0 {
		return candidates
	}
	out := make([]topologyNeighborCandidate, 0, len(candidates))
	out = append(out, candidates[offset:]...)
	out = append(out, candidates[:offset]...)
	return out
}

func topologyRotationOffset(selfPeerID string, bucket string, n int) int {
	if n <= 1 {
		return 0
	}
	h := fnv.New32a()
	_, _ = h.Write([]byte(strings.TrimSpace(selfPeerID)))
	_, _ = h.Write([]byte{0})
	_, _ = h.Write([]byte(strings.TrimSpace(bucket)))
	return int(h.Sum32() % uint32(n))
}

func topologyMembersByPeerID(members []TopologyMember) map[string]TopologyMember {
	out := make(map[string]TopologyMember, len(members))
	for _, mem := range members {
		peerID := strings.TrimSpace(mem.PeerID)
		if peerID != "" {
			out[peerID] = mem
		}
	}
	return out
}

func topologyBucketForPeer(members map[string]TopologyMember, peerID string) string {
	mem, ok := members[strings.TrimSpace(peerID)]
	if !ok {
		return ""
	}
	return pocstate.ReachabilityBucket(mem.V4Hint, mem.V6Hint)
}

func topologyNeighborFailures(attempts []TopologyAttempt, members map[string]TopologyMember) []TopologyNeighborFailure {
	out := make([]TopologyNeighborFailure, 0)
	for _, attempt := range attempts {
		if strings.TrimSpace(attempt.Outcome) == "" || attempt.Outcome == "ok" {
			continue
		}
		peerID := strings.TrimSpace(attempt.PeerID)
		out = append(out, TopologyNeighborFailure{
			PeerID:         peerID,
			Bucket:         topologyBucketForPeer(members, peerID),
			Stage:          strings.TrimSpace(attempt.Stage),
			ReasonCode:     strings.TrimSpace(attempt.ReasonCode),
			ContactedPeers: []string{peerID},
			RetryBudget:    1,
			StopCondition:  strings.TrimSpace(attempt.StopCondition),
		})
	}
	return out
}

func topologyNeighborReplacements(selected []TopologyNeighborSelection, active []TopologyNeighborEdge, unhealthy []TopologyNeighborHealth) []TopologyNeighborReplacement {
	if len(unhealthy) == 0 {
		return []TopologyNeighborReplacement{}
	}

	activePeers := make(map[string]struct{}, len(active))
	for _, edge := range active {
		activePeers[strings.TrimSpace(edge.PeerID)] = struct{}{}
	}
	unhealthyPeers := make(map[string]struct{}, len(unhealthy))
	for _, health := range unhealthy {
		unhealthyPeers[strings.TrimSpace(health.PeerID)] = struct{}{}
	}

	out := make([]TopologyNeighborReplacement, 0)
	for _, health := range unhealthy {
		for _, candidate := range selected {
			peerID := strings.TrimSpace(candidate.PeerID)
			if peerID == "" {
				continue
			}
			if _, ok := activePeers[peerID]; ok {
				continue
			}
			if _, ok := unhealthyPeers[peerID]; ok {
				continue
			}
			out = append(out, TopologyNeighborReplacement{
				OldPeerID: strings.TrimSpace(health.PeerID),
				NewPeerID: peerID,
				Bucket:    candidate.Bucket,
				Reason:    "selected_after_unhealthy_neighbor",
			})
			break
		}
	}
	return out
}

func topologyRecoveryEventsFromMembers(members []TopologyMember) []TopologyRecoveryEvent {
	out := make([]TopologyRecoveryEvent, 0)
	for _, mem := range members {
		if !mem.Revoked {
			continue
		}
		out = append(out, TopologyRecoveryEvent{
			Stage:          "revoke",
			ReasonCode:     "authorization_revocation",
			Message:        "member revoked",
			ContactedPeers: []string{mem.PeerID},
			RetryBudget:    0,
			StopCondition:  "revoked_decl_observed",
		})
	}
	return out
}

func topologyRecoveryEventsFromFailures(failures []TopologyNeighborFailure) []TopologyRecoveryEvent {
	out := make([]TopologyRecoveryEvent, 0, len(failures))
	for _, failure := range failures {
		out = append(out, TopologyRecoveryEvent{
			Stage:          failure.Stage,
			ReasonCode:     failure.ReasonCode,
			Message:        "neighbor attempt failed explainably",
			ContactedPeers: append([]string(nil), failure.ContactedPeers...),
			RetryBudget:    failure.RetryBudget,
			StopCondition:  failure.StopCondition,
		})
	}
	return out
}
