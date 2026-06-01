package pocstate

import (
	"crypto/ed25519"
	"encoding/base64"
	"testing"
)

func TestGovernanceHeadSnapshotV1_ValidateSelfContained_Genesis(t *testing.T) {
	netID := mustTestNetID(t)

	aPriv, aPub, aPubB64 := testEd25519Key(t, 0xA1)
	body := GovernanceSnapshotBodyV1{
		NetID:       netID,
		PrevHashB64: "",
		Height:      0,
		Owners:      []string{aPubB64},
		Admins:      []string{aPubB64},
	}
	sigA := signBody(t, aPriv, aPub, body)

	head := GovernanceHeadSnapshotV1{
		Format:       governanceHeadSnapshotFormatV1,
		SnapshotBody: body,
		Signatures:   []GovernanceSignatureV1{sigA},
	}
	if err := head.ValidateSelfContained(); err != nil {
		t.Fatalf("ValidateSelfContained() error = %v, want nil", err)
	}
}

func TestGovernanceHeadSnapshotV1_ValidateSelfContained_GenesisPrevHashMustBeEmpty(t *testing.T) {
	netID := mustTestNetID(t)

	_, _, aPubB64 := testEd25519Key(t, 0xA1)
	body := GovernanceSnapshotBodyV1{
		NetID:       netID,
		PrevHashB64: "not-empty",
		Height:      0,
		Owners:      []string{aPubB64},
		Admins:      []string{aPubB64},
	}

	head := GovernanceHeadSnapshotV1{
		Format:       governanceHeadSnapshotFormatV1,
		SnapshotBody: body,
	}
	if err := head.ValidateSelfContained(); err == nil {
		t.Fatalf("ValidateSelfContained() error = nil, want error")
	}
}

func TestValidateGovernanceHeadSnapshotBootstrap_NetIDMismatch(t *testing.T) {
	netID := mustTestNetID(t)

	aPriv, aPub, aPubB64 := testEd25519Key(t, 0xA1)
	body := GovernanceSnapshotBodyV1{
		NetID:       netID,
		PrevHashB64: "",
		Height:      0,
		Owners:      []string{aPubB64},
		Admins:      []string{aPubB64},
	}
	sigA := signBody(t, aPriv, aPub, body)

	head := GovernanceHeadSnapshotV1{
		Format:       governanceHeadSnapshotFormatV1,
		SnapshotBody: body,
		Signatures:   []GovernanceSignatureV1{sigA},
	}

	if err := ValidateGovernanceHeadSnapshotBootstrap(netID+"A", head); err == nil {
		t.Fatalf("ValidateGovernanceHeadSnapshotBootstrap() error = nil, want error")
	}
}

func TestValidateGovernanceHeadSnapshotUpdate(t *testing.T) {
	netID := mustTestNetID(t)

	aPriv, aPub, aPubB64 := testEd25519Key(t, 0xA1)
	bPriv, bPub, bPubB64 := testEd25519Key(t, 0xB2)

	localBody := GovernanceSnapshotBodyV1{
		NetID:       netID,
		PrevHashB64: "",
		Height:      0,
		Owners:      []string{aPubB64},
		Admins:      []string{aPubB64},
	}
	localSig := signBody(t, aPriv, aPub, localBody)
	localHead := GovernanceHeadSnapshotV1{
		Format:       governanceHeadSnapshotFormatV1,
		SnapshotBody: localBody,
		Signatures:   []GovernanceSignatureV1{localSig},
	}
	if err := localHead.ValidateSelfContained(); err != nil {
		t.Fatalf("localHead.ValidateSelfContained() error = %v, want nil", err)
	}

	t.Run("overlap_one_sig_ok", func(t *testing.T) {
		candidateBody := GovernanceSnapshotBodyV1{
			NetID:       netID,
			PrevHashB64: localHead.HashB64,
			Height:      1,
			Owners:      []string{aPubB64, bPubB64},
			Admins:      []string{aPubB64},
		}
		candidateSig := signBody(t, aPriv, aPub, candidateBody)
		candidate := GovernanceHeadSnapshotV1{
			Format:       governanceHeadSnapshotFormatV1,
			SnapshotBody: candidateBody,
			Signatures:   []GovernanceSignatureV1{candidateSig},
		}
		if err := ValidateGovernanceHeadSnapshotUpdate(netID, localHead, candidate); err != nil {
			t.Fatalf("ValidateGovernanceHeadSnapshotUpdate() error = %v, want nil", err)
		}
	})

	t.Run("no_overlap_requires_old_and_new_sigs", func(t *testing.T) {
		candidateBody := GovernanceSnapshotBodyV1{
			NetID:       netID,
			PrevHashB64: localHead.HashB64,
			Height:      1,
			Owners:      []string{bPubB64},
			Admins:      []string{aPubB64},
		}
		sigA := signBody(t, aPriv, aPub, candidateBody)
		sigB := signBody(t, bPriv, bPub, candidateBody)
		candidate := GovernanceHeadSnapshotV1{
			Format:       governanceHeadSnapshotFormatV1,
			SnapshotBody: candidateBody,
			Signatures:   []GovernanceSignatureV1{sigA, sigB},
		}
		if err := ValidateGovernanceHeadSnapshotUpdate(netID, localHead, candidate); err != nil {
			t.Fatalf("ValidateGovernanceHeadSnapshotUpdate() error = %v, want nil", err)
		}
	})

	t.Run("fails_old_threshold", func(t *testing.T) {
		candidateBody := GovernanceSnapshotBodyV1{
			NetID:       netID,
			PrevHashB64: localHead.HashB64,
			Height:      1,
			Owners:      []string{bPubB64},
			Admins:      []string{aPubB64},
		}
		sigB := signBody(t, bPriv, bPub, candidateBody)
		candidate := GovernanceHeadSnapshotV1{
			Format:       governanceHeadSnapshotFormatV1,
			SnapshotBody: candidateBody,
			Signatures:   []GovernanceSignatureV1{sigB},
		}
		if err := ValidateGovernanceHeadSnapshotUpdate(netID, localHead, candidate); err == nil {
			t.Fatalf("ValidateGovernanceHeadSnapshotUpdate() error = nil, want error")
		}
	})

	t.Run("fails_prev_mismatch", func(t *testing.T) {
		zeros32 := make([]byte, 32)
		candidateBody := GovernanceSnapshotBodyV1{
			NetID:       netID,
			PrevHashB64: base64.RawURLEncoding.EncodeToString(zeros32),
			Height:      1,
			Owners:      []string{aPubB64},
			Admins:      []string{aPubB64},
		}
		sigA := signBody(t, aPriv, aPub, candidateBody)
		candidate := GovernanceHeadSnapshotV1{
			Format:       governanceHeadSnapshotFormatV1,
			SnapshotBody: candidateBody,
			Signatures:   []GovernanceSignatureV1{sigA},
		}
		if err := ValidateGovernanceHeadSnapshotUpdate(netID, localHead, candidate); err == nil {
			t.Fatalf("ValidateGovernanceHeadSnapshotUpdate() error = nil, want error")
		}
	})

	t.Run("no_op_same_hash_ok", func(t *testing.T) {
		noOp := localHead
		if err := ValidateGovernanceHeadSnapshotUpdate(netID, localHead, noOp); err != nil {
			t.Fatalf("ValidateGovernanceHeadSnapshotUpdate() error = %v, want nil", err)
		}
	})
}

func mustTestNetID(t *testing.T) string {
	t.Helper()

	secret := make([]byte, 32)
	for i := range secret {
		secret[i] = byte(i)
	}
	netID, err := NetIDFromSecret(secret)
	if err != nil {
		t.Fatalf("NetIDFromSecret() error = %v", err)
	}
	return netID
}

func testEd25519Key(t *testing.T, seedByte byte) (ed25519.PrivateKey, ed25519.PublicKey, string) {
	t.Helper()

	seed := make([]byte, 32)
	for i := range seed {
		seed[i] = seedByte
	}
	priv := ed25519.NewKeyFromSeed(seed)
	pub := priv.Public().(ed25519.PublicKey)
	return priv, pub, base64.RawURLEncoding.EncodeToString(pub)
}

func signBody(t *testing.T, priv ed25519.PrivateKey, pub ed25519.PublicKey, body GovernanceSnapshotBodyV1) GovernanceSignatureV1 {
	t.Helper()

	_, _, sum, _, err := snapshotBodyHashV1(body)
	if err != nil {
		t.Fatalf("snapshotBodyHashV1() error = %v", err)
	}
	sig := ed25519.Sign(priv, sum[:])

	return GovernanceSignatureV1{
		KeyID:  keyIDFromEd25519Pub(pub),
		SigB64: base64.RawURLEncoding.EncodeToString(sig),
	}
}
