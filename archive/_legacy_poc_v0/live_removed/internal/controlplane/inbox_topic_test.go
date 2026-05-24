package controlplane

import (
	"strings"
	"testing"
)

func TestDeriveInboxTopic_Deterministic(t *testing.T) {
	netSecret := []byte("0123456789abcdef0123456789abcdef")
	peerID := base32RawNoPad.EncodeToString([]byte("0123456789abcdef"))

	got1, err := DeriveInboxTopic(netSecret, peerID)
	if err != nil {
		t.Fatalf("DeriveInboxTopic(%v, %q) error = %v", netSecret, peerID, err)
	}
	if strings.Contains(got1, strings.ToLower(peerID)) {
		t.Fatalf("DeriveInboxTopic(%v, %q) = %q contains peer_id substring %q", netSecret, peerID, got1, peerID)
	}
	got2, err := DeriveInboxTopic(netSecret, peerID)
	if err != nil {
		t.Fatalf("DeriveInboxTopic(%v, %q) error = %v", netSecret, peerID, err)
	}
	if got1 != got2 {
		t.Fatalf("DeriveInboxTopic(%v, %q) = %q, want %q", netSecret, peerID, got2, got1)
	}
}

func TestDeriveInboxTopic_DifferentPeerIDDifferentTopic(t *testing.T) {
	netSecret := []byte("0123456789abcdef0123456789abcdef")
	peerA := base32RawNoPad.EncodeToString([]byte("0123456789abcdef"))
	peerB := base32RawNoPad.EncodeToString([]byte("0123456789abcdeg"))

	gotA, err := DeriveInboxTopic(netSecret, peerA)
	if err != nil {
		t.Fatalf("DeriveInboxTopic(%v, %q) error = %v", netSecret, peerA, err)
	}
	gotB, err := DeriveInboxTopic(netSecret, peerB)
	if err != nil {
		t.Fatalf("DeriveInboxTopic(%v, %q) error = %v", netSecret, peerB, err)
	}
	if gotA == gotB {
		t.Fatalf("DeriveInboxTopic(%v, peerA) = DeriveInboxTopic(%v, peerB) = %q, want different topics", netSecret, netSecret, gotA)
	}
}

func TestDeriveInboxTopic_CanonicalizesPeerID(t *testing.T) {
	netSecret := []byte("0123456789abcdef0123456789abcdef")
	peerID := base32RawNoPad.EncodeToString([]byte("0123456789abcdef"))
	peerIDVariant := "  " + peerID[:13] + "-" + peerID[13:] + "  "

	got1, err := DeriveInboxTopic(netSecret, peerID)
	if err != nil {
		t.Fatalf("DeriveInboxTopic(%v, %q) error = %v", netSecret, peerID, err)
	}
	got2, err := DeriveInboxTopic(netSecret, peerIDVariant)
	if err != nil {
		t.Fatalf("DeriveInboxTopic(%v, %q) error = %v", netSecret, peerIDVariant, err)
	}
	if got1 != got2 {
		t.Fatalf("DeriveInboxTopic(%v, %q) = %q, want %q", netSecret, peerIDVariant, got2, got1)
	}
}

func TestDeriveInboxTopic_Format(t *testing.T) {
	netSecret := []byte("0123456789abcdef0123456789abcdef")
	peerID := base32RawNoPad.EncodeToString([]byte("0123456789abcdef"))

	topic, err := DeriveInboxTopic(netSecret, peerID)
	if err != nil {
		t.Fatalf("DeriveInboxTopic(%v, %q) error = %v", netSecret, peerID, err)
	}
	if got := len(topic); got != 26 {
		t.Fatalf("DeriveInboxTopic(%v, %q) topic length = %d, want %d", netSecret, peerID, got, 26)
	}
	for _, r := range topic {
		if r >= 'a' && r <= 'z' {
			continue
		}
		if r >= '2' && r <= '7' {
			continue
		}
		t.Fatalf("DeriveInboxTopic(%v, %q) = %q contains invalid character %q", netSecret, peerID, topic, r)
	}
}
