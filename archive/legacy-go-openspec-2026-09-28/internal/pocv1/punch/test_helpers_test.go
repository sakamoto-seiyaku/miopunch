package punch

import (
	"bytes"
	"crypto/ed25519"
	"encoding/base32"
	"testing"

	"github.com/miopunch/miopunch/internal/pocv1/enroll"
	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
)

func mustCanonicalMsgID(t *testing.T, value string) string {
	t.Helper()
	got, err := pocwire.CanonicalizeMsgID(value)
	if err != nil {
		t.Fatalf("CanonicalizeMsgID(%q) error = %v, want nil", value, err)
	}
	return got
}

func mustCanonicalNetworkID(t *testing.T, value string) string {
	t.Helper()
	got, err := pocwire.CanonicalizeNetworkID(value)
	if err != nil {
		t.Fatalf("CanonicalizeNetworkID(%q) error = %v, want nil", value, err)
	}
	return got
}

func mustCanonicalPeerID(t *testing.T, value string) string {
	t.Helper()
	got, err := pocwire.CanonicalizePeerID(value)
	if err != nil {
		t.Fatalf("CanonicalizePeerID(%q) error = %v, want nil", value, err)
	}
	return got
}

func mustMsgIDFromSeed(t *testing.T, seed byte) string {
	t.Helper()
	return mustCanonicalMsgID(t, base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString(bytes.Repeat([]byte{seed}, pocwire.RawIDLen)))
}

type testSigner struct {
	PrivateKey ed25519.PrivateKey
	PublicKey  ed25519.PublicKey
	PeerID     string
}

type testSignedCredential struct {
	Signer     testSigner
	Credential enroll.MemberCredential
	Raw        []byte
}

func mustTestSigner(t *testing.T, seedByte byte) testSigner {
	t.Helper()

	priv := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{seedByte}, ed25519.SeedSize))
	pub := priv.Public().(ed25519.PublicKey)
	peerID, err := pocwire.PeerIDFromEd25519Pub(pub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(seed=%d) error = %v, want nil", seedByte, err)
	}
	return testSigner{
		PrivateKey: priv,
		PublicKey:  append(ed25519.PublicKey(nil), pub...),
		PeerID:     peerID,
	}
}

func mustSignedMemberCredential(
	t *testing.T,
	authorityPriv ed25519.PrivateKey,
	networkID string,
	subjectSeed byte,
) testSignedCredential {
	t.Helper()

	signer := mustTestSigner(t, subjectSeed)
	credential := enroll.MemberCredential{
		NetworkID:         networkID,
		SubjectEd25519Pub: append([]byte(nil), signer.PublicKey...),
		SubjectX25519Pub:  bytes.Repeat([]byte{subjectSeed + 0x11}, 32),
		Role:              "member",
		NotBeforeUnixMs:   1,
		NotAfterUnixMs:    2,
		IssuerKeyID:       "authority-01",
	}
	if err := enroll.SignMemberCredential(authorityPriv, &credential); err != nil {
		t.Fatalf("SignMemberCredential(seed=%d) error = %v, want nil", subjectSeed, err)
	}
	raw, err := credential.MarshalBinary()
	if err != nil {
		t.Fatalf("MemberCredential.MarshalBinary(seed=%d) error = %v, want nil", subjectSeed, err)
	}
	return testSignedCredential{
		Signer:     signer,
		Credential: credential,
		Raw:        raw,
	}
}
