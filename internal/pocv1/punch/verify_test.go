package punch

import (
	"bytes"
	"crypto/ed25519"
	"errors"
	"testing"

	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/presence"
	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
)

func TestVerifyRemoteAssertionRejectsSenderMismatch(t *testing.T) {
	fx := mustVerifyFixture(t)
	inner := fx.inner
	inner.SenderPeerID = mustCanonicalPeerID(t, "AAAAAAAAAAAAAAAAAAAAAAAAAA")
	_, err := verifyRemoteAssertion(fx.cfg, inner, fx.remoteCredentialBytes)
	if !errors.Is(err, ErrRemoteSenderMismatch) {
		t.Fatalf("verifyRemoteAssertion(sender mismatch) error = %v, want %v", err, ErrRemoteSenderMismatch)
	}
}

func TestVerifyRemoteAssertionRejectsRosterMismatch(t *testing.T) {
	fx := mustVerifyFixture(t)
	cfg := fx.cfg
	cfg.TrustedRosterByID[fx.remotePeerID] = persist.RosterEntry{
		PeerID:           fx.remotePeerID,
		MemberCredential: []byte{0x01, 0x02},
	}
	_, err := verifyRemoteAssertion(cfg, fx.inner, fx.remoteCredentialBytes)
	if !errors.Is(err, ErrRemoteRosterMismatch) {
		t.Fatalf("verifyRemoteAssertion(roster mismatch) error = %v, want %v", err, ErrRemoteRosterMismatch)
	}
}

func TestVerifyRemoteAssertionRejectsSenderEd25519Mismatch(t *testing.T) {
	fx := mustVerifyFixture(t)
	inner := fx.inner
	inner.SenderEd25519 = bytes.Repeat([]byte{0x77}, ed25519.PublicKeySize)
	_, err := verifyRemoteAssertion(fx.cfg, inner, fx.remoteCredentialBytes)
	if !errors.Is(err, ErrRemoteSenderMismatch) {
		t.Fatalf("verifyRemoteAssertion(sender ed25519 mismatch) error = %v, want %v", err, ErrRemoteSenderMismatch)
	}
}

func TestVerifyRemoteAssertionRejectsMalformedCredential(t *testing.T) {
	fx := mustVerifyFixture(t)
	_, err := verifyRemoteAssertion(fx.cfg, fx.inner, []byte{0x01, 0x02})
	if !errors.Is(err, ErrRemoteCredentialMismatch) {
		t.Fatalf("verifyRemoteAssertion(malformed credential) error = %v, want %v", err, ErrRemoteCredentialMismatch)
	}
}

func TestVerifyRemoteAssertionRejectsMissingRosterEntry(t *testing.T) {
	fx := mustVerifyFixture(t)
	cfg := fx.cfg
	cfg.TrustedRosterByID = map[string]persist.RosterEntry{}
	_, err := verifyRemoteAssertion(cfg, fx.inner, fx.remoteCredentialBytes)
	if !errors.Is(err, ErrTargetNotInRoster) {
		t.Fatalf("verifyRemoteAssertion(missing roster entry) error = %v, want %v", err, ErrTargetNotInRoster)
	}
}

func TestVerifyRemoteAssertionRejectsAuthorityFailure(t *testing.T) {
	fx := mustVerifyFixture(t)
	cfg := fx.cfg
	cfg.AuthorityEd25519Pub = mustTestSigner(t, 0x55).PublicKey
	_, err := verifyRemoteAssertion(cfg, fx.inner, fx.remoteCredentialBytes)
	if !errors.Is(err, ErrRemoteAuthorityVerify) {
		t.Fatalf("verifyRemoteAssertion(authority failure) error = %v, want %v", err, ErrRemoteAuthorityVerify)
	}
}

func TestVerifyRemoteAssertionAcceptsRosterBackedCredential(t *testing.T) {
	fx := mustVerifyFixture(t)
	got, err := verifyRemoteAssertion(fx.cfg, fx.inner, fx.remoteCredentialBytes)
	if err != nil {
		t.Fatalf("verifyRemoteAssertion() error = %v, want nil", err)
	}
	if got.PeerID != fx.remotePeerID {
		t.Fatalf("verifyRemoteAssertion().PeerID = %q, want %q", got.PeerID, fx.remotePeerID)
	}
	if !bytes.Equal(got.MemberCredential, fx.remoteCredentialBytes) {
		t.Fatalf("verifyRemoteAssertion().MemberCredential mismatch")
	}
}

func TestResolveTargetRejectsInvalidPeerID(t *testing.T) {
	fx := mustVerifyFixture(t)
	_, err := resolveTarget(fx.cfg, Target{PeerID: "bad"})
	if !errors.Is(err, ErrInvalidConfig) {
		t.Fatalf("resolveTarget(invalid peer_id) error = %v, want %v", err, ErrInvalidConfig)
	}
}

func TestResolveTargetRejectsOfflineTarget(t *testing.T) {
	fx := mustVerifyFixture(t)
	cfg := fx.cfg
	cfg.Discover = presence.DiscoverProjection{
		Peers: []presence.DiscoverProjectionPeer{
			{PeerID: fx.remotePeerID, OnlineState: presence.OnlineStateOffline},
		},
	}
	_, err := resolveTarget(cfg, Target{PeerID: fx.remotePeerID})
	if !errors.Is(err, ErrTargetOffline) {
		t.Fatalf("resolveTarget(offline target) error = %v, want %v", err, ErrTargetOffline)
	}
}

func TestResolveTargetRejectsMissingRosterEntry(t *testing.T) {
	fx := mustVerifyFixture(t)
	cfg := fx.cfg
	cfg.TrustedRosterByID = map[string]persist.RosterEntry{}
	_, err := resolveTarget(cfg, Target{PeerID: fx.remotePeerID})
	if !errors.Is(err, ErrTargetNotInRoster) {
		t.Fatalf("resolveTarget(missing roster entry) error = %v, want %v", err, ErrTargetNotInRoster)
	}
}

func TestTrustedRemoteFromRosterRejectsAuthorityFailure(t *testing.T) {
	fx := mustVerifyFixture(t)
	cfg := fx.cfg
	cfg.AuthorityEd25519Pub = mustTestSigner(t, 0x56).PublicKey
	_, err := trustedRemoteFromRoster(cfg, cfg.TrustedRosterByID[fx.remotePeerID])
	if !errors.Is(err, ErrRemoteAuthorityVerify) {
		t.Fatalf("trustedRemoteFromRoster(authority failure) error = %v, want %v", err, ErrRemoteAuthorityVerify)
	}
}

func TestTrustedRemoteFromRosterRejectsPeerIDMismatch(t *testing.T) {
	fx := mustVerifyFixture(t)
	entry := cfgRosterEntryWithPeerID(fx.cfg.TrustedRosterByID[fx.remotePeerID], mustCanonicalPeerID(t, "AAAAAAAAAAAAAAAAAAAAAAAAAA"))
	_, err := trustedRemoteFromRoster(fx.cfg, entry)
	if !errors.Is(err, ErrRemoteRosterMismatch) {
		t.Fatalf("trustedRemoteFromRoster(peer_id mismatch) error = %v, want %v", err, ErrRemoteRosterMismatch)
	}
}

func cfgRosterEntryWithPeerID(entry persist.RosterEntry, peerID string) persist.RosterEntry {
	entry.PeerID = peerID
	return entry
}

type verifyFixture struct {
	cfg                   LoadedConfig
	inner                 pocwire.InnerMessage
	remotePeerID          string
	remoteCredentialBytes []byte
}

func mustVerifyFixture(t *testing.T) verifyFixture {
	t.Helper()

	authority := mustTestSigner(t, 0x10)
	networkID := mustCanonicalNetworkID(t, "LJMVUWK2LJMVUWK2LJMVUWK2LI")
	remoteSigned := mustSignedMemberCredential(t, authority.PrivateKey, networkID, 0x20)

	cfg := LoadedConfig{
		AuthorityEd25519Pub: authority.PublicKey,
		Discover: presence.DiscoverProjection{
			Peers: []presence.DiscoverProjectionPeer{
				{PeerID: remoteSigned.Signer.PeerID, OnlineState: presence.OnlineStateOnline},
			},
		},
		TrustedRosterByID: map[string]persist.RosterEntry{
			remoteSigned.Signer.PeerID: {
				PeerID:           remoteSigned.Signer.PeerID,
				MemberCredential: remoteSigned.Raw,
			},
		},
	}

	msgID := mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA")
	inner := pocwire.InnerMessage{
		DstPeerID:       remoteSigned.Signer.PeerID,
		MsgID:           msgID,
		CreatedAtUnixMs: 1,
		ExpiresAtUnixMs: 2,
		SenderPeerID:    remoteSigned.Signer.PeerID,
		SenderEd25519:   append([]byte(nil), remoteSigned.Signer.PublicKey...),
		Kind:            pocwire.KindDialOffer,
		Body:            []byte{0x01},
		Signature:       bytes.Repeat([]byte{0x22}, ed25519.SignatureSize),
	}

	return verifyFixture{
		cfg:                   cfg,
		inner:                 inner,
		remotePeerID:          remoteSigned.Signer.PeerID,
		remoteCredentialBytes: remoteSigned.Raw,
	}
}
