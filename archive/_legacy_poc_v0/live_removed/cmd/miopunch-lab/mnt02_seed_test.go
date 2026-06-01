package main

import (
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/miopunch/miopunch/internal/pocstate"
)

func TestRunMNT02Seed_DoesNotBootstrapMembershipState(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	out, err := runMNT02Seed(mnt02SeedConfig{
		StateRoot: filepath.Join(dir, "state"),
		PeerCount: 3,

		MQTTBroker:  "127.0.0.1:1883",
		TopicPrefix: "miopunch/mnt02",
		ProxyName:   "mnt02",
		SecretKey:   "mnt02-secret",
		DataProto:   "quic",
		QUICCC:      "bbr",
		P2PNetwork:  "auto",
		P2PIPFamily: "auto",
		P2PPortBase: 5000,
		P2PPortStep: 1,

		StunServers: []string{"127.0.0.1:3478"},
		DisableStun: false,

		DisablePortMap: true,
	})
	if err != nil {
		t.Fatalf("runMNT02Seed() error = %v", err)
	}

	if out.Format != mnt02SeedFormatV0 {
		t.Fatalf("runMNT02Seed().Format = %q, want %q", out.Format, mnt02SeedFormatV0)
	}
	if out.PeerCount != 3 {
		t.Fatalf("runMNT02Seed().PeerCount = %d, want %d", out.PeerCount, 3)
	}
	if out.InjectedAllowed["identity"] != true {
		t.Errorf("runMNT02Seed().InjectedAllowed[identity] = %v, want true", out.InjectedAllowed["identity"])
	}
	if out.InjectedAllowed["peer_config"] != true {
		t.Errorf("runMNT02Seed().InjectedAllowed[peer_config] = %v, want true", out.InjectedAllowed["peer_config"])
	}

	if len(out.Peers) != 3 {
		t.Fatalf("runMNT02Seed().Peers = %d, want %d", len(out.Peers), 3)
	}

	for _, p := range out.Peers {
		if _, err := os.Stat(p.StatePath); err != nil {
			t.Fatalf("peer %d state missing: %v", p.Index, err)
		}
		stateDir := filepath.Dir(p.StatePath)
		if _, err := os.Stat(filepath.Join(stateDir, "identity", "identity.json")); err != nil {
			t.Fatalf("peer %d identity missing: %v", p.Index, err)
		}

		// Seed MUST NOT bootstrap net/governance/decls membership state.
		if _, err := os.Stat(filepath.Join(stateDir, "net.json")); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("peer %d net.json exists or unexpected error: %v", p.Index, err)
		}
		if _, err := os.Stat(filepath.Join(stateDir, "governance", "head_snapshot.json")); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("peer %d head_snapshot.json exists or unexpected error: %v", p.Index, err)
		}
		if _, err := os.Stat(filepath.Join(stateDir, "decls", "decls.json")); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("peer %d decls.json exists or unexpected error: %v", p.Index, err)
		}

		st, err := pocstate.Load(p.StatePath)
		if err != nil {
			t.Fatalf("peer %d load state: %v", p.Index, err)
		}
		if st.Local == nil {
			t.Fatalf("peer %d local is nil", p.Index)
		}
		if st.Local.StunExplicit != true {
			t.Fatalf("peer %d local.stun_explicit = %v, want true", p.Index, st.Local.StunExplicit)
		}
		if len(st.Peers) != 0 {
			t.Fatalf("peer %d state.peers = %v, want empty", p.Index, st.Peers)
		}
	}
}

func TestValidateMNT02SeedConfig_RejectsPublicBrokerDefault(t *testing.T) {
	t.Parallel()

	cfg := mnt02SeedConfig{
		StateRoot:   t.TempDir(),
		PeerCount:   2,
		MQTTBroker:  "mqtt.eclipseprojects.io:1883",
		TopicPrefix: "miopunch/mnt02",
		ProxyName:   "mnt02",
		SecretKey:   "mnt02-secret",
		P2PPortBase: 5000,
		P2PPortStep: 1,
	}
	if err := validateMNT02SeedConfig(cfg); err == nil {
		t.Fatalf("validateMNT02SeedConfig() error = nil, want error")
	}
}
