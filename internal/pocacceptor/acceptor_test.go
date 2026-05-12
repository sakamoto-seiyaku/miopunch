package pocacceptor

import (
	"errors"
	"path/filepath"
	"strings"
	"testing"

	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func TestSafeStreamMetadataSummaryRedactsSensitiveValues(t *testing.T) {
	got := safeStreamMetadataSummary(map[string]string{
		"op":           "ping",
		"task_id":      "task-sh-001",
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
		`task_id="task-sh-001"`,
		`peer_id="peer-1"`,
	} {
		if !strings.Contains(got, needle) {
			t.Fatalf("safeStreamMetadataSummary(...) = %q, want %q", got, needle)
		}
	}
}

func TestShellAttachWaitFailure_ClassifiesLayerAndReason(t *testing.T) {
	t.Parallel()

	err := errors.New("process exited: 255")
	tests := []struct {
		name       string
		target     string
		wantReason string
		wantLayer  string
		wantPrefix string
	}{
		{
			name:       "local tmux",
			target:     "local",
			wantReason: "SH_TMUX_ATTACH_FAIL",
			wantLayer:  "tmux",
			wantPrefix: "tmux session exited:",
		},
		{
			name:       "ssh target",
			target:     "ssh:ops",
			wantReason: "SH_CONNECTOR_FAIL",
			wantLayer:  "ssh",
			wantPrefix: "ssh process exited:",
		},
		{
			name:       "wsl target",
			target:     "wsl:Ubuntu",
			wantReason: "SH_CONNECTOR_FAIL",
			wantLayer:  "wsl",
			wantPrefix: "wsl process exited:",
		},
		{
			name:       "other target",
			target:     "custom",
			wantReason: "SH_CONNECTOR_FAIL",
			wantLayer:  "shelltarget",
			wantPrefix: "shell target exited:",
		},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			got := shellAttachWaitFailure(tt.target, err)
			if got.reasonCode != tt.wantReason {
				t.Fatalf("shellAttachWaitFailure(%q).reasonCode = %q, want %q", tt.target, got.reasonCode, tt.wantReason)
			}
			if got.shellLayer != tt.wantLayer {
				t.Fatalf("shellAttachWaitFailure(%q).shellLayer = %q, want %q", tt.target, got.shellLayer, tt.wantLayer)
			}
			if !strings.HasPrefix(got.message, tt.wantPrefix) {
				t.Fatalf("shellAttachWaitFailure(%q).message = %q, want prefix %q", tt.target, got.message, tt.wantPrefix)
			}
			if len(got.suggestions) == 0 || got.suggestions[0] != "retry" {
				t.Fatalf("shellAttachWaitFailure(%q).suggestions = %v, want retry", tt.target, got.suggestions)
			}
		})
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
