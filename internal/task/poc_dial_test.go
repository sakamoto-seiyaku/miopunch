package task

import (
	"path/filepath"
	"testing"

	"github.com/miopunch/miopunch/internal/pocstate"
)

func TestLoadPeerConfigUsesLocalDialDefaults(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	st := pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			MQTTBroker:           "100.65.0.10:1883",
			TopicPrefix:          "miopunch/test",
			DataProto:            "kcp",
			QUICCC:               "brutal",
			P2PNetwork:           "udp_only",
			P2PIPFamily:          "v4",
			P2PPort:              5000,
			StunServers:          []string{"100.65.0.11:3478"},
			StunExplicit:         true,
			DisablePortMap:       true,
			DisableAssistedAddrs: true,
		},
		Peers: map[string]pocstate.PeerConfig{
			"peer-a": {
				ProxyName:   "peer-a",
				SecretKey:   "secret",
				MQTTBroker:  "100.65.0.10:1883",
				TopicPrefix: "miopunch/test",
			},
		},
	}
	if err := pocstate.Save(statePath, st); err != nil {
		t.Fatalf("pocstate.Save() error = %v", err)
	}

	m := NewManagerWithStatePath(statePath)
	cfg, ok, err := m.loadPeerConfig("peer-a")
	if err != nil {
		t.Fatalf("loadPeerConfig(peer-a) error = %v", err)
	}
	if !ok {
		t.Fatal("loadPeerConfig(peer-a) ok = false, want true")
	}

	if cfg.P2PNetwork != "udp_only" {
		t.Errorf("loadPeerConfig(peer-a).P2PNetwork = %q, want %q", cfg.P2PNetwork, "udp_only")
	}
	if cfg.P2PIPFamily != "v4" {
		t.Errorf("loadPeerConfig(peer-a).P2PIPFamily = %q, want %q", cfg.P2PIPFamily, "v4")
	}
	if cfg.P2PPort != 0 {
		t.Errorf("loadPeerConfig(peer-a).P2PPort = %d, want %d", cfg.P2PPort, 0)
	}
	if len(cfg.StunServers) != 1 || cfg.StunServers[0] != "100.65.0.11:3478" {
		t.Errorf("loadPeerConfig(peer-a).StunServers = %v, want [100.65.0.11:3478]", cfg.StunServers)
	}
	if !cfg.StunExplicit {
		t.Error("loadPeerConfig(peer-a).StunExplicit = false, want true")
	}
	if !cfg.DisablePortMap {
		t.Error("loadPeerConfig(peer-a).DisablePortMap = false, want true")
	}
	if !cfg.DisableAssistedAddrs {
		t.Error("loadPeerConfig(peer-a).DisableAssistedAddrs = false, want true")
	}
}

func TestLoadPeerConfigKeepsPeerDialOverrides(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	st := pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			P2PNetwork:  "udp_only",
			P2PIPFamily: "v4",
			P2PPort:     5000,
			StunServers: []string{"100.65.0.11:3478"},
		},
		Peers: map[string]pocstate.PeerConfig{
			"peer-a": {
				ProxyName:    "peer-a",
				SecretKey:    "secret",
				MQTTBroker:   "100.65.0.10:1883",
				TopicPrefix:  "miopunch/test",
				P2PNetwork:   "tcp_only",
				P2PIPFamily:  "v6",
				P2PPort:      6000,
				StunServers:  []string{"100.65.0.99:3478"},
				StunExplicit: true,
			},
		},
	}
	if err := pocstate.Save(statePath, st); err != nil {
		t.Fatalf("pocstate.Save() error = %v", err)
	}

	m := NewManagerWithStatePath(statePath)
	cfg, ok, err := m.loadPeerConfig("peer-a")
	if err != nil {
		t.Fatalf("loadPeerConfig(peer-a) error = %v", err)
	}
	if !ok {
		t.Fatal("loadPeerConfig(peer-a) ok = false, want true")
	}

	if cfg.P2PNetwork != "tcp_only" {
		t.Errorf("loadPeerConfig(peer-a).P2PNetwork = %q, want %q", cfg.P2PNetwork, "tcp_only")
	}
	if cfg.P2PIPFamily != "v6" {
		t.Errorf("loadPeerConfig(peer-a).P2PIPFamily = %q, want %q", cfg.P2PIPFamily, "v6")
	}
	if cfg.P2PPort != 6000 {
		t.Errorf("loadPeerConfig(peer-a).P2PPort = %d, want %d", cfg.P2PPort, 6000)
	}
	if len(cfg.StunServers) != 1 || cfg.StunServers[0] != "100.65.0.99:3478" {
		t.Errorf("loadPeerConfig(peer-a).StunServers = %v, want [100.65.0.99:3478]", cfg.StunServers)
	}
}
