package task

import (
	"bytes"
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
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

func TestRunBootstrapMoreUsesAvailableEffectiveBroker(t *testing.T) {
	testCases := []struct {
		name string
		pair func(reachable string, unreachable string) []string
	}{
		{
			name: "falls back to secondary effective broker",
			pair: func(reachable string, unreachable string) []string {
				return []string{unreachable, reachable}
			},
		},
		{
			name: "keeps primary when secondary is unreachable",
			pair: func(reachable string, unreachable string) []string {
				return []string{reachable, unreachable}
			},
		},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			reachable := startTCPMQTTBroker(t)
			unreachable := unusedLocalTCPAddr(t)
			brokers := tc.pair(reachable, unreachable)

			requesterStatePath := filepath.Join(t.TempDir(), "requester", "state.json")
			responderStatePath := filepath.Join(t.TempDir(), "responder", "state.json")

			requesterStateDir, err := pocstate.StateDir(requesterStatePath)
			if err != nil {
				t.Fatalf("pocstate.StateDir(%q) error = %v", requesterStatePath, err)
			}
			requesterID, err := pocstate.EnsureIdentity(requesterStateDir)
			if err != nil {
				t.Fatalf("pocstate.EnsureIdentity(%q) error = %v", requesterStateDir, err)
			}

			responderStateDir, err := pocstate.StateDir(responderStatePath)
			if err != nil {
				t.Fatalf("pocstate.StateDir(%q) error = %v", responderStatePath, err)
			}
			responderID, err := pocstate.EnsureIdentity(responderStateDir)
			if err != nil {
				t.Fatalf("pocstate.EnsureIdentity(%q) error = %v", responderStateDir, err)
			}

			netSecret := bytes.Repeat([]byte{7}, 32)
			approved := []pocstate.Identity{requesterID, responderID}

			saveBootstrapMoreStateForTest(t, requesterStatePath, netSecret, requesterID, approved, brokers, nil)
			saveBootstrapMoreStateForTest(t, responderStatePath, netSecret, requesterID, approved, brokers, map[string]pocstate.PeerConfig{
				"peer-candidate": {
					ProxyName:   "peer-candidate",
					SecretKey:   "peer-candidate-secret",
					MQTTBroker:  "broker:1883",
					TopicPrefix: "miopunch/test",
					V4Hint:      "easy",
					V6Hint:      "none",
				},
			})

			requester := NewManagerWithStatePath(requesterStatePath)
			t.Cleanup(requester.Close)
			responder := NewManagerWithStatePath(responderStatePath)
			t.Cleanup(responder.Close)

			responderRaw, err := json.Marshal(BootstrapMoreArgs{
				Mode:    "respond_once",
				Timeout: "10s",
			})
			if err != nil {
				t.Fatalf("json.Marshal(responder args) error = %v", err)
			}
			responderTask, err := responder.CreateAndRun(CreateRequest{
				Kind: "bootstrap_more",
				Args: responderRaw,
			})
			if err != nil {
				t.Fatalf("responder.CreateAndRun(bootstrap_more) error = %v", err)
			}
			waitTaskStageForTest(t, responder, responderTask.ID, poc.StagePeerContact)

			requesterRaw, err := json.Marshal(BootstrapMoreArgs{
				TargetPeerID: responderID.PeerID,
				Round:        1,
				Timeout:      "5s",
			})
			if err != nil {
				t.Fatalf("json.Marshal(requester args) error = %v", err)
			}
			requesterTask, err := requester.CreateAndRun(CreateRequest{
				Kind: "bootstrap_more",
				Args: requesterRaw,
			})
			if err != nil {
				t.Fatalf("requester.CreateAndRun(bootstrap_more) error = %v", err)
			}

			requesterFinal := waitTaskDoneForTest(t, requester, requesterTask.ID)
			if requesterFinal.ReasonCode != poc.ReasonCodeOK {
				t.Fatalf("request bootstrap_more ReasonCode = %q, want %q; facts=%v", requesterFinal.ReasonCode, poc.ReasonCodeOK, requesterFinal.Facts)
			}
			responderFinal := waitTaskDoneForTest(t, responder, responderTask.ID)
			if responderFinal.ReasonCode != poc.ReasonCodeOK {
				t.Fatalf("respond_once bootstrap_more ReasonCode = %q, want %q; facts=%v", responderFinal.ReasonCode, poc.ReasonCodeOK, responderFinal.Facts)
			}

			if !taskFactsContainSubstring(requesterFinal, "mqtt broker skipped: "+unreachable+":") {
				t.Errorf("request bootstrap_more facts = %v, want skipped broker diagnostic for %q", requesterFinal.Facts, unreachable)
			}
			if !taskFactsContainSubstring(responderFinal, "mqtt broker skipped: "+unreachable+":") {
				t.Errorf("respond_once bootstrap_more facts = %v, want skipped broker diagnostic for %q", responderFinal.Facts, unreachable)
			}
			if !taskFactsContain(requesterFinal, "bootstrap_more_candidates=1") {
				t.Errorf("request bootstrap_more facts = %v, want bootstrap_more_candidates=1", requesterFinal.Facts)
			}
			if !taskFactsContain(responderFinal, "bootstrap_more_candidates=1") {
				t.Errorf("respond_once bootstrap_more facts = %v, want bootstrap_more_candidates=1", responderFinal.Facts)
			}
		})
	}
}

func TestRunBootstrapMoreFailsWhenAllEffectiveBrokersAreUnavailable(t *testing.T) {
	unreachableA := unusedLocalTCPAddr(t)
	unreachableB := unusedLocalTCPAddr(t)
	statePath := filepath.Join(t.TempDir(), "requester", "state.json")

	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	requesterID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(%q) error = %v", stateDir, err)
	}

	otherStateDir := t.TempDir()
	targetID, err := pocstate.EnsureIdentity(otherStateDir)
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(%q) error = %v", otherStateDir, err)
	}

	saveBootstrapMoreStateForTest(
		t,
		statePath,
		bytes.Repeat([]byte{9}, 32),
		requesterID,
		[]pocstate.Identity{requesterID, targetID},
		[]string{unreachableA, unreachableB},
		nil,
	)

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	raw, err := json.Marshal(BootstrapMoreArgs{
		TargetPeerID: targetID.PeerID,
		Round:        1,
		Timeout:      "5s",
	})
	if err != nil {
		t.Fatalf("json.Marshal(BootstrapMoreArgs) error = %v", err)
	}
	created, err := m.CreateAndRun(CreateRequest{Kind: "bootstrap_more", Args: raw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(bootstrap_more) error = %v", err)
	}

	final := waitTaskDoneForTest(t, m, created.ID)
	if final.ReasonCode != poc.ReasonCodeUnavailable {
		t.Fatalf("bootstrap_more ReasonCode = %q, want %q; facts=%v", final.ReasonCode, poc.ReasonCodeUnavailable, final.Facts)
	}
	if !taskFactsContainSubstring(final, "mqtt broker skipped: "+unreachableA+":") {
		t.Errorf("bootstrap_more facts = %v, want skipped broker diagnostic for %q", final.Facts, unreachableA)
	}
	if !taskFactsContainSubstring(final, "mqtt broker skipped: "+unreachableB+":") {
		t.Errorf("bootstrap_more facts = %v, want skipped broker diagnostic for %q", final.Facts, unreachableB)
	}
	if !taskFactsContainSubstring(final, "mqtt connect failed: "+unreachableA+":") {
		t.Errorf("bootstrap_more facts = %v, want mqtt connect failed fact containing %q", final.Facts, unreachableA)
	}
	if taskFactsContainPrefix(final, "bootstrap_more_response_id=") {
		t.Errorf("bootstrap_more facts = %v, want no response id when all effective brokers fail", final.Facts)
	}
}

func saveBootstrapMoreStateForTest(
	t *testing.T,
	statePath string,
	netSecret []byte,
	issuer pocstate.Identity,
	approved []pocstate.Identity,
	brokers []string,
	peers map[string]pocstate.PeerConfig,
) {
	t.Helper()

	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}

	if err := pocstate.SaveNet(stateDir, pocstate.Net{
		NetSecret:        append([]byte(nil), netSecret...),
		BrokersEffective: append([]string(nil), brokers...),
	}); err != nil {
		t.Fatalf("pocstate.SaveNet(%q) error = %v", stateDir, err)
	}
	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		t.Fatalf("pocstate.EnsureDecls(%q) error = %v", stateDir, err)
	}
	if _, err := pocstate.UpdateDecls(stateDir, func(f *pocstate.DeclsFileV0) error {
		for _, member := range approved {
			f.Decls = pocstate.AddDeclSetUnionV0(f.Decls, mustApproveDecl(t, issuer, member, "unknown"))
		}
		return nil
	}); err != nil {
		t.Fatalf("pocstate.UpdateDecls(%q) error = %v", stateDir, err)
	}
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Peers:  peers,
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}
}

func waitTaskStageForTest(t *testing.T, m *Manager, taskID string, want poc.Stage) Task {
	t.Helper()

	deadline := time.After(5 * time.Second)
	ticker := time.NewTicker(10 * time.Millisecond)
	defer ticker.Stop()

	for {
		if got, ok := m.Get(taskID); ok {
			if got.Stage == want {
				return got
			}
			if got.Status == StatusDone {
				t.Fatalf("Manager.Get(%q) reached done before stage %q: %#v", taskID, want, got)
			}
		}

		select {
		case <-deadline:
			t.Fatalf("Manager.Get(%q) did not reach stage %q", taskID, want)
		case <-ticker.C:
		}
	}
}
