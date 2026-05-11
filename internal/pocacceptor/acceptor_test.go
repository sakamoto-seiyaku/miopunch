package pocacceptor

import (
	"path/filepath"
	"strings"
	"testing"

	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func TestSafeStreamMetadataSummaryRedactsSensitiveValues(t *testing.T) {
	got := safeStreamMetadataSummary(map[string]string{
		"op":           "ping",
		"peer_id":      "peer-1",
		"seed_peer":    `{"secret_key":"SECRET_SHOULD_NOT_LEAK"}`,
		"approve_decl": `{"body":"DECL_SHOULD_NOT_LEAK"}`,
		"decls":        `[{"body":"DECLS_SHOULD_NOT_LEAK"}]`,
		"target":       "shell",
		"session":      "main",
	})

	for _, needle := range []string{
		"SECRET_SHOULD_NOT_LEAK",
		"secret_key",
		"DECL_SHOULD_NOT_LEAK",
		"DECLS_SHOULD_NOT_LEAK",
	} {
		if strings.Contains(got, needle) {
			t.Fatalf("safeStreamMetadataSummary(...) = %q, want no %q", got, needle)
		}
	}
	for _, needle := range []string{
		"seed_peer_present=true",
		"approve_decl_present=true",
		"decls_present=true",
		`op="ping"`,
		`peer_id="peer-1"`,
	} {
		if !strings.Contains(got, needle) {
			t.Fatalf("safeStreamMetadataSummary(...) = %q, want %q", got, needle)
		}
	}
}

func TestSafeAcceptorMQTTSummaryRedactsTopicPrefix(t *testing.T) {
	local := &pocstate.LocalConfig{
		MQTTBroker:  "broker-1.miopunch.local:1883",
		TopicPrefix: "miopunch/private/topic",
		DataProto:   "quic",
		QUICCC:      "bbr",
		P2PNetwork:  "udp",
		P2PIPFamily: "dual",
		P2PPort:     47888,
	}

	got := safeAcceptorMQTTSummary(local)

	for _, needle := range []string{
		"miopunch/private/topic",
		"topic_prefix",
	} {
		if strings.Contains(got, needle) {
			t.Fatalf("safeAcceptorMQTTSummary(...) = %q, want no %q", got, needle)
		}
	}
	for _, needle := range []string{
		"broker=broker-1.miopunch.local:1883",
		"data_proto=quic",
		"quic_cc=bbr",
		"p2p_network=udp",
		"p2p_ip_family=dual",
		"p2p_port=47888",
	} {
		if !strings.Contains(got, needle) {
			t.Fatalf("safeAcceptorMQTTSummary(...) = %q, want %q", got, needle)
		}
	}
}

func TestPersistHelloSeedPeerStoresBrokerList(t *testing.T) {
	stateDir := t.TempDir()
	id, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity() error = %v", err)
	}
	seed := &shellproto.PeerSeed{
		PeerID:      id.PeerID,
		ProxyName:   id.PeerID,
		SecretKey:   "peer-1-secret",
		MQTTBroker:  "broker-a:1883",
		MQTTBrokers: []string{"broker-a:1883", "broker-b:1883"},
		TopicPrefix: "miopunch/test",
		DataProto:   "quic",
		QUICCC:      "bbr",
	}

	if err := persistHelloSeedPeer(stateDir, id.PeerID, seed); err != nil {
		t.Fatalf("persistHelloSeedPeer() error = %v", err)
	}

	st, err := pocstate.Load(filepath.Join(stateDir, "state.json"))
	if err != nil {
		t.Fatalf("pocstate.Load(state.json) error = %v", err)
	}
	cfg, ok := st.Peers[id.PeerID]
	if !ok {
		t.Fatalf("state peers missing %s: %#v", id.PeerID, st.Peers)
	}
	got := cfg.MQTTBrokerEndpoints()
	want := []string{"broker-a:1883", "broker-b:1883"}
	if len(got) != len(want) || got[0] != want[0] || got[1] != want[1] {
		t.Fatalf("persistHelloSeedPeer() saved brokers = %v, want %v", got, want)
	}
}
