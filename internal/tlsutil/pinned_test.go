package tlsutil

import (
	"bytes"
	"crypto/ed25519"
	"crypto/tls"
	"net"
	"testing"
	"time"
)

func TestDerivePinnedEd25519PublicKey_Deterministic(t *testing.T) {
	t.Parallel()

	secret := []byte("secret")
	sid := "sid-1"

	a1, err := DerivePinnedEd25519PublicKey(secret, sid, "visitor")
	if err != nil {
		t.Fatalf("DerivePinnedEd25519PublicKey() error = %v", err)
	}
	a2, err := DerivePinnedEd25519PublicKey(secret, sid, "visitor")
	if err != nil {
		t.Fatalf("DerivePinnedEd25519PublicKey() error = %v", err)
	}
	if !bytes.Equal(a1, a2) {
		t.Fatalf("pinned public keys differ for same inputs")
	}

	otherRole, err := DerivePinnedEd25519PublicKey(secret, sid, "client")
	if err != nil {
		t.Fatalf("DerivePinnedEd25519PublicKey() error = %v", err)
	}
	if bytes.Equal(a1, otherRole) {
		t.Fatalf("expected different key for different role")
	}

	otherSid, err := DerivePinnedEd25519PublicKey(secret, "sid-2", "visitor")
	if err != nil {
		t.Fatalf("DerivePinnedEd25519PublicKey() error = %v", err)
	}
	if bytes.Equal(a1, otherSid) {
		t.Fatalf("expected different key for different sid")
	}

	otherSecret, err := DerivePinnedEd25519PublicKey([]byte("other"), sid, "visitor")
	if err != nil {
		t.Fatalf("DerivePinnedEd25519PublicKey() error = %v", err)
	}
	if bytes.Equal(a1, otherSecret) {
		t.Fatalf("expected different key for different secret")
	}

	if len(a1) != ed25519.PublicKeySize {
		t.Fatalf("unexpected key size: %d", len(a1))
	}
}

func TestPinnedTLSHandshake_RejectsMismatch(t *testing.T) {
	secret := []byte("secret")
	sid := "sid-tls"

	clientCfg, err := NewPinnedClientTLSConfig(secret, sid, "visitor", "client")
	if err != nil {
		t.Fatalf("NewPinnedClientTLSConfig() error = %v", err)
	}
	serverCfg, err := NewPinnedServerTLSConfig(secret, sid, "client", "visitor")
	if err != nil {
		t.Fatalf("NewPinnedServerTLSConfig() error = %v", err)
	}

	c1, s1 := net.Pipe()
	defer c1.Close()
	defer s1.Close()

	client := tls.Client(c1, clientCfg)
	server := tls.Server(s1, serverCfg)

	errCh := make(chan error, 2)
	go func() { errCh <- client.Handshake() }()
	go func() { errCh <- server.Handshake() }()

	timeout := time.NewTimer(2 * time.Second)
	defer timeout.Stop()

	for i := 0; i < 2; i++ {
		select {
		case err := <-errCh:
			if err != nil {
				t.Fatalf("handshake error = %v, want nil", err)
			}
		case <-timeout.C:
			t.Fatalf("handshake timeout")
		}
	}
	if !timeout.Stop() {
		select {
		case <-timeout.C:
		default:
		}
	}

	// Now fail with a mismatched secret.
	badClientCfg, err := NewPinnedClientTLSConfig([]byte("wrong"), sid, "visitor", "client")
	if err != nil {
		t.Fatalf("NewPinnedClientTLSConfig() error = %v", err)
	}

	c2, s2 := net.Pipe()
	defer c2.Close()
	defer s2.Close()

	badClient := tls.Client(c2, badClientCfg)
	server2 := tls.Server(s2, serverCfg)
	_ = badClient.SetDeadline(time.Now().Add(800 * time.Millisecond))
	_ = server2.SetDeadline(time.Now().Add(800 * time.Millisecond))

	errCh = make(chan error, 2)
	go func() { errCh <- badClient.Handshake() }()
	go func() { errCh <- server2.Handshake() }()

	var failed int
	timeout = time.NewTimer(2 * time.Second)
	defer timeout.Stop()
	for i := 0; i < 2; i++ {
		select {
		case err := <-errCh:
			if err != nil {
				failed++
			}
		case <-timeout.C:
			t.Fatalf("handshake timeout (mismatch)")
		}
	}
	if failed == 0 {
		t.Fatalf("expected handshake failure for mismatched pinned identity")
	}
}
