package pocstate

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/base32"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"
)

const (
	netFormatV0 = "miopunch.net.poc.v0"
)

var (
	base32RawNoPadNet = base32.StdEncoding.WithPadding(base32.NoPadding)
)

type netRecordV0 struct {
	Format           string   `json:"format"`
	CreatedAtUnixMs  int64    `json:"created_at_unix_ms"`
	NetID            string   `json:"net_id"`
	NetSecretB64     string   `json:"net_secret_b64"`
	BrokersEffective []string `json:"brokers_effective,omitempty"`
}

type Net struct {
	NetID     string
	NetSecret []byte

	BrokersEffective []string
}

func netPath(stateDir string) string {
	return filepath.Join(stateDir, "net.json")
}

func LoadNet(stateDir string) (Net, error) {
	if strings.TrimSpace(stateDir) == "" {
		return Net{}, errors.New("empty state_dir")
	}

	data, err := os.ReadFile(netPath(stateDir))
	if err != nil {
		return Net{}, err
	}

	var rec netRecordV0
	if err := json.Unmarshal(data, &rec); err != nil {
		return Net{}, fmt.Errorf("unmarshal net: %w", err)
	}
	if strings.TrimSpace(rec.Format) == "" {
		rec.Format = netFormatV0
	}
	if rec.Format != netFormatV0 {
		return Net{}, fmt.Errorf("unsupported net format: %q", rec.Format)
	}

	secret, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(rec.NetSecretB64))
	if err != nil {
		return Net{}, fmt.Errorf("decode net_secret_b64: %w", err)
	}
	if len(secret) != 32 {
		return Net{}, fmt.Errorf("invalid net_secret length: %d", len(secret))
	}

	netID, err := netIDFromSecret(secret)
	if err != nil {
		return Net{}, err
	}
	if strings.TrimSpace(rec.NetID) != "" && netID != strings.TrimSpace(rec.NetID) {
		return Net{}, fmt.Errorf("net_id mismatch: have=%q want=%q", rec.NetID, netID)
	}

	out := Net{
		NetID:            netID,
		NetSecret:        secret,
		BrokersEffective: normalizeBrokers(rec.BrokersEffective),
	}
	return out, nil
}

func SaveNet(stateDir string, n Net) error {
	if strings.TrimSpace(stateDir) == "" {
		return errors.New("empty state_dir")
	}
	if len(n.NetSecret) != 32 {
		return fmt.Errorf("invalid net_secret length: %d", len(n.NetSecret))
	}
	netID, err := netIDFromSecret(n.NetSecret)
	if err != nil {
		return err
	}

	rec := netRecordV0{
		Format:           netFormatV0,
		CreatedAtUnixMs:  time.Now().UTC().UnixMilli(),
		NetID:            netID,
		NetSecretB64:     base64.RawURLEncoding.EncodeToString(n.NetSecret),
		BrokersEffective: normalizeBrokers(n.BrokersEffective),
	}

	data, err := json.MarshalIndent(rec, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal net: %w", err)
	}
	data = append(data, '\n')

	path := netPath(stateDir)
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return fmt.Errorf("mkdir state_dir: %w", err)
	}
	if err := writeFileAtomic(path, data, 0o600); err != nil {
		return fmt.Errorf("write net: %w", err)
	}
	return nil
}

func EnsureNet(stateDir string, brokersEffective []string) (Net, error) {
	n, err := LoadNet(stateDir)
	if err == nil {
		return n, nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return Net{}, err
	}

	secret := make([]byte, 32)
	if _, err := rand.Read(secret); err != nil {
		return Net{}, fmt.Errorf("read net_secret: %w", err)
	}
	netID, err := netIDFromSecret(secret)
	if err != nil {
		return Net{}, err
	}

	n = Net{
		NetID:            netID,
		NetSecret:        secret,
		BrokersEffective: normalizeBrokers(brokersEffective),
	}

	rec := netRecordV0{
		Format:           netFormatV0,
		CreatedAtUnixMs:  time.Now().UTC().UnixMilli(),
		NetID:            n.NetID,
		NetSecretB64:     base64.RawURLEncoding.EncodeToString(secret),
		BrokersEffective: n.BrokersEffective,
	}
	data, err := json.MarshalIndent(rec, "", "  ")
	if err != nil {
		return Net{}, fmt.Errorf("marshal net: %w", err)
	}
	data = append(data, '\n')

	path := netPath(stateDir)
	if err := writeFileExclusive(path, data, 0o600); err != nil {
		if errors.Is(err, os.ErrExist) {
			return LoadNet(stateDir)
		}
		return Net{}, fmt.Errorf("write net: %w", err)
	}

	return n, nil
}

func netIDFromSecret(netSecret []byte) (string, error) {
	if len(netSecret) == 0 {
		return "", errors.New("net_secret is required")
	}
	sum := sha256.Sum256(netSecret)
	id := strings.ToUpper(base32RawNoPadNet.EncodeToString(sum[:16]))
	if len(id) != 26 {
		return "", fmt.Errorf("unexpected net_id length: %d", len(id))
	}
	return id, nil
}

func NetIDFromSecret(netSecret []byte) (string, error) {
	return netIDFromSecret(netSecret)
}

func normalizeBrokers(in []string) []string {
	out := make([]string, 0, len(in))
	seen := make(map[string]struct{}, len(in))
	for _, raw := range in {
		ep := strings.TrimSpace(raw)
		if ep == "" {
			continue
		}
		if _, ok := seen[ep]; ok {
			continue
		}
		seen[ep] = struct{}{}
		out = append(out, ep)
	}
	return out
}
