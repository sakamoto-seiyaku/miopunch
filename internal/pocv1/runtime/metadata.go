package runtime

import (
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/miopunch/miopunch/internal/atomicfile"
)

type metadata struct {
	ActiveNetworkID        string `json:"active_network_id,omitempty"`
	AuthorityEd25519PubB64 string `json:"authority_ed25519_pub_b64,omitempty"`
	AuthorityX25519PubB64  string `json:"authority_x25519_pub_b64,omitempty"`
	Role                   string `json:"role,omitempty"`
	BrokerEndpoint         string `json:"broker_endpoint,omitempty"`
	RuntimeBrokerOverride  string `json:"runtime_broker_override,omitempty"`
}

func metadataPath(root string) string {
	return filepath.Join(root, "runtime_v1.json")
}

func loadMetadata(root string) (metadata, error) {
	path := metadataPath(root)
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return metadata{}, nil
		}
		return metadata{}, fmt.Errorf("read runtime metadata: %w", err)
	}

	var meta metadata
	if err := json.Unmarshal(data, &meta); err != nil {
		return metadata{}, fmt.Errorf("decode runtime metadata: %w", err)
	}
	return meta, nil
}

func saveMetadata(root string, meta metadata) error {
	data, err := json.MarshalIndent(meta, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal runtime metadata: %w", err)
	}
	data = append(data, '\n')
	if err := atomicfile.WriteFile(metadataPath(root), data, 0o600); err != nil {
		return fmt.Errorf("write runtime metadata: %w", err)
	}
	return nil
}

func encodeKeyB64(data []byte) string {
	if len(data) == 0 {
		return ""
	}
	return base64.RawURLEncoding.EncodeToString(data)
}

func decodeKeyB64(value string) ([]byte, error) {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil, nil
	}
	data, err := base64.RawURLEncoding.DecodeString(value)
	if err != nil {
		return nil, err
	}
	return data, nil
}

func (m metadata) authorityEd25519() (ed25519.PublicKey, error) {
	data, err := decodeKeyB64(m.AuthorityEd25519PubB64)
	if err != nil {
		return nil, fmt.Errorf("decode authority ed25519 public key: %w", err)
	}
	if len(data) == 0 {
		return nil, nil
	}
	if len(data) != ed25519.PublicKeySize {
		return nil, fmt.Errorf("invalid authority ed25519 public key length: %d", len(data))
	}
	return ed25519.PublicKey(append([]byte(nil), data...)), nil
}

func (m metadata) authorityX25519() ([]byte, error) {
	data, err := decodeKeyB64(m.AuthorityX25519PubB64)
	if err != nil {
		return nil, fmt.Errorf("decode authority x25519 public key: %w", err)
	}
	if len(data) == 0 {
		return nil, nil
	}
	if len(data) != 32 {
		return nil, fmt.Errorf("invalid authority x25519 public key length: %d", len(data))
	}
	return append([]byte(nil), data...), nil
}
