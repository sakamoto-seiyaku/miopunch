package controlplane

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/hkdf"
	"crypto/rand"
	"crypto/sha256"
	"errors"
	"fmt"
	"strings"

	"golang.org/x/crypto/curve25519"
)

const (
	inviteAEADVersionV0 = 0
	inviteAEADNonceLen  = 12

	inviteJoinRequestInfoV0 = "miopunch/v0/aead.invite.join_request"
)

var (
	ErrInviteCiphertextInvalid = errors.New("invalid invite ciphertext")
	ErrInviteCiphertextVersion = errors.New("unsupported invite ciphertext version")
)

func deriveInviteKeyV0(inviteSecret []byte, inviteTopic string, info string) ([]byte, error) {
	if len(inviteSecret) != 32 {
		return nil, fmt.Errorf("invalid invite_secret length: %d", len(inviteSecret))
	}
	inviteTopic = strings.TrimSpace(inviteTopic)
	if inviteTopic == "" {
		return nil, errors.New("empty invite_topic")
	}
	if strings.TrimSpace(info) == "" {
		return nil, errors.New("empty invite hkdf info")
	}

	saltSum := sha256.Sum256([]byte(inviteTopic))
	key, err := hkdf.Key(sha256.New, inviteSecret, saltSum[:16], info, 32)
	if err != nil {
		return nil, fmt.Errorf("derive invite key: %w", err)
	}
	if len(key) != 32 {
		return nil, fmt.Errorf("unexpected invite key length: %d", len(key))
	}
	return key, nil
}

func SealInviteJoinRequestV0(inviteSecret []byte, inviteTopic string, plaintext []byte) ([]byte, error) {
	key, err := deriveInviteKeyV0(inviteSecret, inviteTopic, inviteJoinRequestInfoV0)
	if err != nil {
		return nil, err
	}
	return sealInviteFrameV0(key, plaintext)
}

func OpenInviteJoinRequestV0(inviteSecret []byte, inviteTopic string, frame []byte) ([]byte, error) {
	key, err := deriveInviteKeyV0(inviteSecret, inviteTopic, inviteJoinRequestInfoV0)
	if err != nil {
		return nil, err
	}
	return openInviteFrameV0(key, frame)
}

func SealInviteMembershipBundleV0(issuerX25519Priv []byte, memberX25519Pub []byte, inviteTopic string, issuerPeerID string, memberPeerID string, plaintext []byte) ([]byte, error) {
	key, err := deriveMembershipBundleKeyV0(issuerX25519Priv, memberX25519Pub, inviteTopic, issuerPeerID, memberPeerID)
	if err != nil {
		return nil, err
	}
	return sealInviteFrameV0(key, plaintext)
}

func OpenInviteMembershipBundleV0(memberX25519Priv []byte, issuerX25519Pub []byte, inviteTopic string, issuerPeerID string, memberPeerID string, frame []byte) ([]byte, error) {
	key, err := deriveMembershipBundleKeyV0(memberX25519Priv, issuerX25519Pub, inviteTopic, issuerPeerID, memberPeerID)
	if err != nil {
		return nil, err
	}
	return openInviteFrameV0(key, frame)
}

func deriveMembershipBundleKeyV0(priv []byte, pub []byte, inviteTopic string, issuerPeerID string, memberPeerID string) ([]byte, error) {
	if len(priv) != 32 {
		return nil, fmt.Errorf("invalid x25519 priv length: %d", len(priv))
	}
	if len(pub) != 32 {
		return nil, fmt.Errorf("invalid x25519 pub length: %d", len(pub))
	}
	inviteTopic = strings.TrimSpace(inviteTopic)
	if inviteTopic == "" {
		return nil, errors.New("empty invite_topic")
	}

	issuerPeerID, err := CanonicalizePeerID(issuerPeerID)
	if err != nil {
		return nil, fmt.Errorf("invalid issuer_peer_id: %w", err)
	}
	memberPeerID, err = CanonicalizePeerID(memberPeerID)
	if err != nil {
		return nil, fmt.Errorf("invalid member_peer_id: %w", err)
	}

	shared, err := curve25519.X25519(priv, pub)
	if err != nil {
		return nil, fmt.Errorf("x25519: %w", err)
	}
	if len(shared) != 32 {
		return nil, fmt.Errorf("unexpected x25519 shared length: %d", len(shared))
	}

	info := fmt.Sprintf("miopunch/v0/aead.invite.membership_bundle/%s/%s", issuerPeerID, memberPeerID)
	saltSum := sha256.Sum256([]byte(inviteTopic))
	key, err := hkdf.Key(sha256.New, shared, saltSum[:16], info, 32)
	if err != nil {
		return nil, fmt.Errorf("derive membership key: %w", err)
	}
	if len(key) != 32 {
		return nil, fmt.Errorf("unexpected membership key length: %d", len(key))
	}
	return key, nil
}

func sealInviteFrameV0(key []byte, plaintext []byte) ([]byte, error) {
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, fmt.Errorf("new cipher: %w", err)
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, fmt.Errorf("new gcm: %w", err)
	}
	if gcm.NonceSize() != inviteAEADNonceLen {
		return nil, fmt.Errorf("unexpected gcm nonce size: %d", gcm.NonceSize())
	}

	nonce := make([]byte, inviteAEADNonceLen)
	if _, err := rand.Read(nonce); err != nil {
		return nil, fmt.Errorf("read nonce: %w", err)
	}
	ct := gcm.Seal(nil, nonce, plaintext, nil)

	frame := make([]byte, 1+inviteAEADNonceLen+len(ct))
	frame[0] = inviteAEADVersionV0
	copy(frame[1:], nonce)
	copy(frame[1+inviteAEADNonceLen:], ct)
	return frame, nil
}

func openInviteFrameV0(key []byte, frame []byte) ([]byte, error) {
	if len(frame) < 1+inviteAEADNonceLen+16 {
		return nil, ErrInviteCiphertextInvalid
	}
	if frame[0] != inviteAEADVersionV0 {
		return nil, ErrInviteCiphertextVersion
	}
	nonce := frame[1 : 1+inviteAEADNonceLen]
	ct := frame[1+inviteAEADNonceLen:]

	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, fmt.Errorf("new cipher: %w", err)
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, fmt.Errorf("new gcm: %w", err)
	}
	if gcm.NonceSize() != inviteAEADNonceLen {
		return nil, fmt.Errorf("unexpected gcm nonce size: %d", gcm.NonceSize())
	}

	pt, err := gcm.Open(nil, nonce, ct, nil)
	if err != nil {
		return nil, fmt.Errorf("open gcm: %w", err)
	}
	return pt, nil
}
