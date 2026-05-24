package controlplane

import (
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"testing"
)

func testBase32ID(label string) string {
	sum := sha256.Sum256([]byte(label))
	return base32RawNoPad.EncodeToString(sum[:16])
}

func TestSignVerifyV0_SignatureCoverage(t *testing.T) {
	seed := []byte("0123456789abcdef0123456789abcdef")
	priv := ed25519.NewKeyFromSeed(seed)
	pub := priv.Public().(ed25519.PublicKey)

	body, err := json.Marshal(struct {
		N int `json:"n"`
	}{N: 1})
	if err != nil {
		t.Fatalf("json.Marshal(body) error = %v", err)
	}

	dstPeerID := testBase32ID("dst")
	senderPeerID := testBase32ID("sender")
	msgID := testBase32ID("msg-1")

	m := Message{
		ProtoVersion: ProtoVersionV0,
		Route: Route{
			DstPeerID:       dstPeerID,
			MsgID:           msgID,
			HopLimit:        HopLimitMax,
			CreatedAtUnixMs: 123,
		},
		Signed: Signed{
			SenderPeerID: senderPeerID,
			Kind:         "smoke_echo_req",
			Body:         body,
		},
	}

	if err := SignV0(priv, &m); err != nil {
		t.Fatalf("SignV0() error = %v", err)
	}
	if err := VerifyV0(pub, m); err != nil {
		t.Fatalf("VerifyV0() error = %v", err)
	}

	m2 := m
	m2.Route.HopLimit = HopLimitMax - 1
	if err := VerifyV0(pub, m2); err != nil {
		t.Fatalf("VerifyV0(hop_limit changed) error = %v", err)
	}

	m3 := m
	m3.Route.DstPeerID = testBase32ID("dst-2")
	if err := VerifyV0(pub, m3); !errors.Is(err, ErrInvalidSignature) {
		t.Fatalf("VerifyV0(dst_peer_id changed) error = %v, want %v", err, ErrInvalidSignature)
	}
}

func TestVerifyV0ForSelf_EnforcesDstPeerID(t *testing.T) {
	seed := []byte("0123456789abcdef0123456789abcdef")
	priv := ed25519.NewKeyFromSeed(seed)
	pub := priv.Public().(ed25519.PublicKey)

	body, err := json.Marshal(struct {
		OK bool `json:"ok"`
	}{OK: true})
	if err != nil {
		t.Fatalf("json.Marshal(body) error = %v", err)
	}

	selfPeerID := testBase32ID("self")
	senderPeerID := testBase32ID("sender")
	msgID := testBase32ID("msg-2")

	m := Message{
		ProtoVersion: ProtoVersionV0,
		Route: Route{
			DstPeerID:       selfPeerID,
			MsgID:           msgID,
			HopLimit:        HopLimitMax,
			CreatedAtUnixMs: 123,
		},
		Signed: Signed{
			SenderPeerID: senderPeerID,
			Kind:         "smoke_echo_resp",
			Body:         body,
		},
	}
	if err := SignV0(priv, &m); err != nil {
		t.Fatalf("SignV0() error = %v", err)
	}

	if err := VerifyV0ForSelf(selfPeerID, pub, m); err != nil {
		t.Fatalf("VerifyV0ForSelf(self) error = %v", err)
	}

	other := testBase32ID("other")
	if err := VerifyV0ForSelf(other, pub, m); !errors.Is(err, ErrNotForSelf) {
		t.Fatalf("VerifyV0ForSelf(other) error = %v, want %v", err, ErrNotForSelf)
	}
}
