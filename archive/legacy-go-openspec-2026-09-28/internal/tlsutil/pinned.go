package tlsutil

import (
	"crypto/ed25519"
	"crypto/hkdf"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"errors"
	"fmt"
	"math/big"
	"strings"
	"time"
)

const pinnedTLSHKDFInfoPrefix = "miopunch/v0/tls.pinned.ed25519/"

func DerivePinnedEd25519PrivateKey(secretKey []byte, sid string, role string) (ed25519.PrivateKey, error) {
	role = strings.TrimSpace(strings.ToLower(role))
	if role == "" {
		return nil, errors.New("role is required")
	}
	if len(secretKey) == 0 {
		return nil, errors.New("secret_key is required")
	}
	if strings.TrimSpace(sid) == "" {
		return nil, errors.New("sid is required")
	}

	seed, err := hkdf.Key(sha256.New, secretKey, []byte(sid), pinnedTLSHKDFInfoPrefix+role, ed25519.SeedSize)
	if err != nil {
		return nil, fmt.Errorf("derive pinned tls seed: %w", err)
	}
	return ed25519.NewKeyFromSeed(seed), nil
}

func DerivePinnedEd25519PublicKey(secretKey []byte, sid string, role string) (ed25519.PublicKey, error) {
	priv, err := DerivePinnedEd25519PrivateKey(secretKey, sid, role)
	if err != nil {
		return nil, err
	}
	pub, ok := priv.Public().(ed25519.PublicKey)
	if !ok {
		return nil, errors.New("unexpected public key type")
	}
	return pub, nil
}

func NewPinnedTLSCertificate(secretKey []byte, sid string, role string) (tls.Certificate, error) {
	priv, err := DerivePinnedEd25519PrivateKey(secretKey, sid, role)
	if err != nil {
		return tls.Certificate{}, err
	}
	return NewEd25519SelfSignedTLSCertificate(priv, role)
}

// NewEd25519SelfSignedTLSCertificate returns a self-signed TLS certificate for priv.
func NewEd25519SelfSignedTLSCertificate(priv ed25519.PrivateKey, role string) (tls.Certificate, error) {
	if len(priv) != ed25519.PrivateKeySize {
		return tls.Certificate{}, fmt.Errorf("invalid ed25519 private key length: %d", len(priv))
	}
	now := time.Now()
	template := &x509.Certificate{
		SerialNumber: big.NewInt(1),
		Subject: pkix.Name{
			CommonName: "miopunch-" + strings.TrimSpace(strings.ToLower(role)),
		},
		NotBefore:             now.Add(-1 * time.Hour),
		NotAfter:              now.Add(3650 * 24 * time.Hour),
		KeyUsage:              x509.KeyUsageDigitalSignature,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth, x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: true,
	}

	der, err := x509.CreateCertificate(rand.Reader, template, template, priv.Public(), priv)
	if err != nil {
		return tls.Certificate{}, fmt.Errorf("create pinned tls cert: %w", err)
	}

	return tls.Certificate{
		Certificate: [][]byte{der},
		PrivateKey:  priv,
	}, nil
}

// PeerEd25519PublicKey parses rawCerts[0] and returns a copy of its Ed25519 public key.
func PeerEd25519PublicKey(rawCerts [][]byte) (ed25519.PublicKey, error) {
	if len(rawCerts) == 0 {
		return nil, errors.New("peer cert missing")
	}

	cert, err := x509.ParseCertificate(rawCerts[0])
	if err != nil {
		return nil, fmt.Errorf("parse peer cert: %w", err)
	}

	pub, ok := cert.PublicKey.(ed25519.PublicKey)
	if !ok {
		return nil, errors.New("peer public key must be ed25519")
	}
	if len(pub) != ed25519.PublicKeySize {
		return nil, fmt.Errorf("unexpected peer public key length: %d", len(pub))
	}
	return append(ed25519.PublicKey(nil), pub...), nil
}

func NewPinnedClientTLSConfig(secretKey []byte, sid string, selfRole string, peerRole string) (*tls.Config, error) {
	cert, err := NewPinnedTLSCertificate(secretKey, sid, selfRole)
	if err != nil {
		return nil, err
	}
	expectedPeerPub, err := DerivePinnedEd25519PublicKey(secretKey, sid, peerRole)
	if err != nil {
		return nil, err
	}

	verifyPeer := pinnedVerifyPeerCertificate(expectedPeerPub)
	return &tls.Config{
		MinVersion:         tls.VersionTLS13,
		MaxVersion:         tls.VersionTLS13,
		InsecureSkipVerify: true,
		Certificates:       []tls.Certificate{cert},
		NextProtos:         []string{"miopunch-tls-stream"},
		VerifyPeerCertificate: func(rawCerts [][]byte, verifiedChains [][]*x509.Certificate) error {
			_ = verifiedChains
			return verifyPeer(rawCerts)
		},
	}, nil
}

func NewPinnedServerTLSConfig(secretKey []byte, sid string, selfRole string, peerRole string) (*tls.Config, error) {
	cert, err := NewPinnedTLSCertificate(secretKey, sid, selfRole)
	if err != nil {
		return nil, err
	}
	expectedPeerPub, err := DerivePinnedEd25519PublicKey(secretKey, sid, peerRole)
	if err != nil {
		return nil, err
	}

	verifyPeer := pinnedVerifyPeerCertificate(expectedPeerPub)
	return &tls.Config{
		MinVersion:   tls.VersionTLS13,
		MaxVersion:   tls.VersionTLS13,
		ClientAuth:   tls.RequireAnyClientCert,
		Certificates: []tls.Certificate{cert},
		NextProtos:   []string{"miopunch-tls-stream"},
		VerifyPeerCertificate: func(rawCerts [][]byte, verifiedChains [][]*x509.Certificate) error {
			_ = verifiedChains
			return verifyPeer(rawCerts)
		},
	}, nil
}

func pinnedVerifyPeerCertificate(expected ed25519.PublicKey) func(rawCerts [][]byte) error {
	expected = append(ed25519.PublicKey(nil), expected...)
	return func(rawCerts [][]byte) error {
		pub, err := PeerEd25519PublicKey(rawCerts)
		if err != nil {
			return err
		}

		if subtle.ConstantTimeCompare(pub, expected) != 1 {
			return errors.New("pinned peer identity mismatch")
		}
		return nil
	}
}
