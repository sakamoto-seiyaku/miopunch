package task

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/pocstate"
)

func TestBootstrapRecommendationsReturnTwoCandidatesWhenKnownPeerExists(t *testing.T) {
	st := pocstate.State{
		Local: &pocstate.LocalConfig{
			ProxyName:   "self",
			SecretKey:   "self-secret",
			MQTTBroker:  "broker:1883",
			TopicPrefix: "miopunch/test",
			DataProto:   "quic",
			QUICCC:      "bbr",
		},
		Peers: map[string]pocstate.PeerConfig{
			"peer-b": {
				ProxyName:   "peer-b",
				SecretKey:   "peer-b-secret",
				MQTTBroker:  "broker:1883",
				TopicPrefix: "miopunch/test",
			},
			"peer-c": {
				ProxyName:   "peer-c",
				SecretKey:   "peer-c-secret",
				MQTTBroker:  "broker:1883",
				TopicPrefix: "miopunch/test",
			},
		},
	}

	got := bootstrapRecommendations("self", "peer-c", st)
	if len(got) != 2 {
		t.Fatalf("bootstrapRecommendations(self, peer-c) length = %d, want 2: %#v", len(got), got)
	}
	if got[0].PeerID != "self" {
		t.Errorf("bootstrapRecommendations(self, peer-c)[0].PeerID = %q, want self", got[0].PeerID)
	}
	if got[1].PeerID != "peer-b" {
		t.Errorf("bootstrapRecommendations(self, peer-c)[1].PeerID = %q, want peer-b", got[1].PeerID)
	}
}

func TestSeedPeersForRecommendationsOnlyIncludesUsableConfigs(t *testing.T) {
	st := pocstate.State{
		Local: &pocstate.LocalConfig{
			ProxyName:   "self",
			SecretKey:   "self-secret",
			MQTTBroker:  "broker:1883",
			TopicPrefix: "miopunch/test",
		},
		Peers: map[string]pocstate.PeerConfig{
			"peer-a": {
				ProxyName:   "peer-a",
				SecretKey:   "peer-a-secret",
				MQTTBroker:  "broker:1883",
				TopicPrefix: "miopunch/test",
			},
			"peer-b": {
				ProxyName: "peer-b",
			},
		},
	}
	recs := []pocstate.BootstrapPeerEvidenceV0{
		{PeerID: "self"},
		{PeerID: "peer-a"},
		{PeerID: "peer-b"},
	}

	got := seedPeersForRecommendations(recs, "self", st)
	if len(got) != 2 {
		t.Fatalf("seedPeersForRecommendations(...) length = %d, want 2: %#v", len(got), got)
	}
	if got[0].PeerID != "self" || got[1].PeerID != "peer-a" {
		t.Errorf("seedPeersForRecommendations(...) peer IDs = [%s %s], want [self peer-a]", got[0].PeerID, got[1].PeerID)
	}
}

func TestBootstrapRecommendationsSortKnownPeersByReachabilityBucket(t *testing.T) {
	st := pocstate.State{
		Local: &pocstate.LocalConfig{
			ProxyName:   "self",
			SecretKey:   "self-secret",
			MQTTBroker:  "broker:1883",
			TopicPrefix: "miopunch/test",
		},
		Peers: map[string]pocstate.PeerConfig{
			"peer-hard": {
				ProxyName:   "peer-hard",
				SecretKey:   "peer-hard-secret",
				MQTTBroker:  "broker:1883",
				TopicPrefix: "miopunch/test",
				V4Hint:      "hard1",
				V6Hint:      "none",
			},
			"peer-direct": {
				ProxyName:   "peer-direct",
				SecretKey:   "peer-direct-secret",
				MQTTBroker:  "broker:1883",
				TopicPrefix: "miopunch/test",
				V4Hint:      "direct",
				V6Hint:      "none",
			},
		},
	}

	got := bootstrapRecommendations("self", "joiner", st)
	if len(got) != 2 {
		t.Fatalf("bootstrapRecommendations(self, joiner) length = %d, want 2: %#v", len(got), got)
	}
	if got[1].PeerID != "peer-direct" {
		t.Errorf("bootstrapRecommendations(self, joiner)[1].PeerID = %q, want peer-direct", got[1].PeerID)
	}
	if got[1].Bucket != "direct" {
		t.Errorf("bootstrapRecommendations(self, joiner)[1].Bucket = %q, want direct", got[1].Bucket)
	}
}

func TestBootstrapRecommendationsPreferIPv6WithinSameBucket(t *testing.T) {
	st := pocstate.State{
		Local: &pocstate.LocalConfig{
			ProxyName:   "self",
			SecretKey:   "self-secret",
			MQTTBroker:  "broker:1883",
			TopicPrefix: "miopunch/test",
		},
		Peers: map[string]pocstate.PeerConfig{
			"peer-a-v4-direct": {
				ProxyName:   "peer-a-v4-direct",
				SecretKey:   "peer-a-v4-direct-secret",
				MQTTBroker:  "broker:1883",
				TopicPrefix: "miopunch/test",
				V4Hint:      "direct",
				V6Hint:      "none",
			},
			"peer-z-v6-direct": {
				ProxyName:   "peer-z-v6-direct",
				SecretKey:   "peer-z-v6-direct-secret",
				MQTTBroker:  "broker:1883",
				TopicPrefix: "miopunch/test",
				V4Hint:      "hard1",
				V6Hint:      "direct",
			},
		},
	}

	got := bootstrapRecommendations("self", "joiner", st)
	if len(got) != 2 {
		t.Fatalf("bootstrapRecommendations(self, joiner) length = %d, want 2: %#v", len(got), got)
	}
	if got[1].PeerID != "peer-z-v6-direct" {
		t.Errorf("bootstrapRecommendations(self, joiner)[1].PeerID = %q, want peer-z-v6-direct", got[1].PeerID)
	}
	if got[1].Bucket != "direct" {
		t.Errorf("bootstrapRecommendations(self, joiner)[1].Bucket = %q, want direct", got[1].Bucket)
	}
}

func TestBootstrapMoreCandidatesDeduplicateAndRespectBuckets(t *testing.T) {
	st := pocstate.State{
		Peers: map[string]pocstate.PeerConfig{
			"peer-hard": {
				ProxyName:   "peer-hard",
				SecretKey:   "peer-hard-secret",
				MQTTBroker:  "broker:1883",
				TopicPrefix: "miopunch/test",
				V4Hint:      "hard1",
				V6Hint:      "none",
			},
			"peer-direct": {
				ProxyName:   "peer-direct",
				SecretKey:   "peer-direct-secret",
				MQTTBroker:  "broker:1883",
				TopicPrefix: "miopunch/test",
				V4Hint:      "direct",
				V6Hint:      "none",
			},
			"peer-easy": {
				ProxyName:   "peer-easy",
				SecretKey:   "peer-easy-secret",
				MQTTBroker:  "broker:1883",
				TopicPrefix: "miopunch/test",
				V4Hint:      "easy",
				V6Hint:      "none",
			},
			"peer-broken": {
				ProxyName: "peer-broken",
			},
		},
	}

	got := bootstrapMoreCandidates("self", "requester", []string{"peer-direct", "peer-direct"}, st)
	if len(got) != 2 {
		t.Fatalf("bootstrapMoreCandidates(...) length = %d, want 2: %#v", len(got), got)
	}
	if got[0].PeerID != "peer-easy" || got[0].Bucket != "easy" {
		t.Errorf("bootstrapMoreCandidates(...)[0] = %#v, want peer-easy/easy", got[0])
	}
	if got[1].PeerID != "peer-hard" || got[1].Bucket != "hard1" {
		t.Errorf("bootstrapMoreCandidates(...)[1] = %#v, want peer-hard/hard1", got[1])
	}
}

func TestBootstrapMoreCandidatesReportsExhausted(t *testing.T) {
	st := pocstate.State{
		Peers: map[string]pocstate.PeerConfig{
			"peer-direct": {
				ProxyName:   "peer-direct",
				SecretKey:   "peer-direct-secret",
				MQTTBroker:  "broker:1883",
				TopicPrefix: "miopunch/test",
				V4Hint:      "direct",
				V6Hint:      "none",
			},
		},
	}

	got := bootstrapMoreCandidates("self", "requester", []string{"peer-direct"}, st)
	if len(got) != 0 {
		t.Fatalf("bootstrapMoreCandidates(exhausted) length = %d, want 0: %#v", len(got), got)
	}
}

func TestNewBootstrapMoreRequestIsBoundedAndMetadataOnly(t *testing.T) {
	stateDir := t.TempDir()
	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity() error = %v", err)
	}

	msg, err := newBootstrapMoreRequest(selfID, selfID.PeerID, 2, []string{selfID.PeerID}, 5*time.Second)
	if err != nil {
		t.Fatalf("newBootstrapMoreRequest() error = %v", err)
	}
	if msg.Signed.Kind != bootstrapMoreRequestKindV0 {
		t.Fatalf("newBootstrapMoreRequest().Signed.Kind = %q, want %q", msg.Signed.Kind, bootstrapMoreRequestKindV0)
	}
	if msg.Route.ExpiresAtUnixMs <= msg.Route.CreatedAtUnixMs {
		t.Fatalf("newBootstrapMoreRequest() expires_at = %d, want after created_at %d", msg.Route.ExpiresAtUnixMs, msg.Route.CreatedAtUnixMs)
	}

	var body bootstrapMoreRequestBodyV0
	if err := json.Unmarshal(msg.Signed.Body, &body); err != nil {
		t.Fatalf("json.Unmarshal(request body) error = %v", err)
	}
	if body.Round != 2 {
		t.Errorf("newBootstrapMoreRequest() body.Round = %d, want 2", body.Round)
	}
	if len(body.Failures) != 1 || body.Failures[0].Reason != "candidate_exhausted" {
		t.Errorf("newBootstrapMoreRequest() body.Failures = %#v, want coarse candidate_exhausted failure", body.Failures)
	}

}
