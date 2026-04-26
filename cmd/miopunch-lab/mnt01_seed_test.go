package main

import (
	"path/filepath"
	"testing"
)

func TestRunMNT01Seed_DisclosesAuthBootstrap(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	out, err := runMNT01Seed(mnt01SeedConfig{
		StateA:          filepath.Join(dir, "a", "state.json"),
		StateB:          filepath.Join(dir, "b", "state.json"),
		MQTTBroker:      "127.0.0.1:1883",
		ProxyName:       "mnt01",
		SecretKey:       "mnt01-secret",
		DataProto:       "quic",
		QUICCC:          "bbr",
		P2PNetwork:      "auto",
		P2PIPFamilyA:    "auto",
		P2PIPFamilyB:    "auto",
		P2PPortA:        5000,
		P2PPortB:        5000,
		DialPortA:       5001,
		DialPortB:       5001,
		StunServers:     []string{"127.0.0.1:3478"},
		DisableStun:     false,
		DisablePortMapA: true,
		DisablePortMapB: true,
	})
	if err != nil {
		t.Fatalf("runMNT01Seed() error = %v", err)
	}

	if out.AuthBootstrap["purpose"] != "hello_auth_only" {
		t.Errorf("runMNT01Seed().AuthBootstrap[purpose] = %v, want hello_auth_only", out.AuthBootstrap["purpose"])
	}
	if out.AuthBootstrap["governance_head_snapshot"] != true {
		t.Errorf("runMNT01Seed().AuthBootstrap[governance_head_snapshot] = %v, want true", out.AuthBootstrap["governance_head_snapshot"])
	}
	if out.AuthBootstrap["approve_member_decl"] != true {
		t.Errorf("runMNT01Seed().AuthBootstrap[approve_member_decl] = %v, want true", out.AuthBootstrap["approve_member_decl"])
	}
	if out.InjectedAllowed["auth_bootstrap"] != true {
		t.Errorf("runMNT01Seed().InjectedAllowed[auth_bootstrap] = %v, want true", out.InjectedAllowed["auth_bootstrap"])
	}
	if !containsString(out.NotInjected, "candidate_path") {
		t.Errorf("runMNT01Seed().NotInjected = %v, want candidate_path", out.NotInjected)
	}
}

func containsString(values []string, want string) bool {
	for _, value := range values {
		if value == want {
			return true
		}
	}
	return false
}
