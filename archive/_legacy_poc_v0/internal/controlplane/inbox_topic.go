package controlplane

import (
	"crypto/hkdf"
	"crypto/sha256"
	"errors"
	"fmt"
	"strings"
)

const (
	inboxTopicHKDFInfoPrefix = "miopunch/v0/topic.inbox/"
)

// DeriveInboxTopic derives the peer's MQTT inbox topic from netSecret and
// peerID, per POC v0 requirements:
//
// - net_id_raw16 = sha256(net_secret)[:16]
// - name16 = HKDF(net_secret, salt=net_id_raw16, info="miopunch/v0/topic.inbox/"+peer_id, L=16)
// - inbox_topic = base32(raw,no-pad,name16) and normalized to lower-case
func DeriveInboxTopic(netSecret []byte, peerID string) (string, error) {
	if len(netSecret) == 0 {
		return "", errors.New("net_secret is required")
	}

	canonicalPeerID, err := CanonicalizePeerID(peerID)
	if err != nil {
		return "", err
	}

	netIDRaw := sha256.Sum256(netSecret)
	name16, err := hkdf.Key(sha256.New, netSecret, netIDRaw[:16], inboxTopicHKDFInfoPrefix+canonicalPeerID, 16)
	if err != nil {
		return "", fmt.Errorf("derive inbox topic: %w", err)
	}

	topic := strings.ToLower(base32RawNoPad.EncodeToString(name16))
	if len(topic) != 26 {
		return "", fmt.Errorf("unexpected inbox topic length: %d", len(topic))
	}
	return topic, nil
}
