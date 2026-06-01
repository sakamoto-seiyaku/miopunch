package pocstate

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const bootstrapFormatV0 = "miopunch.bootstrap.poc.v0"

type BootstrapPeerEvidenceV0 struct {
	PeerID string `json:"peer_id"`
	Bucket string `json:"bucket,omitempty"`
	Reason string `json:"reason,omitempty"`
}

type BootstrapFileV0 struct {
	Format          string                    `json:"format"`
	Recommendations []BootstrapPeerEvidenceV0 `json:"recommendations"`
	Attempts        []BootstrapPeerEvidenceV0 `json:"attempts,omitempty"`
	MoreRounds      []BootstrapPeerEvidenceV0 `json:"more_rounds,omitempty"`
}

func bootstrapPath(stateDir string) string {
	return filepath.Join(stateDir, "bootstrap", "bootstrap.json")
}

func LoadBootstrap(stateDir string) (BootstrapFileV0, error) {
	if strings.TrimSpace(stateDir) == "" {
		return BootstrapFileV0{}, errors.New("empty state_dir")
	}

	data, err := os.ReadFile(bootstrapPath(stateDir))
	if err != nil {
		return BootstrapFileV0{}, err
	}

	var f BootstrapFileV0
	if err := json.Unmarshal(data, &f); err != nil {
		return BootstrapFileV0{}, fmt.Errorf("unmarshal bootstrap: %w", err)
	}
	if strings.TrimSpace(f.Format) == "" {
		f.Format = bootstrapFormatV0
	}
	if f.Format != bootstrapFormatV0 {
		return BootstrapFileV0{}, fmt.Errorf("unsupported bootstrap format: %q", f.Format)
	}
	if f.Recommendations == nil {
		f.Recommendations = []BootstrapPeerEvidenceV0{}
	}
	if f.Attempts == nil {
		f.Attempts = []BootstrapPeerEvidenceV0{}
	}
	if f.MoreRounds == nil {
		f.MoreRounds = []BootstrapPeerEvidenceV0{}
	}
	return f, nil
}

func SaveBootstrap(stateDir string, f BootstrapFileV0) error {
	if strings.TrimSpace(stateDir) == "" {
		return errors.New("empty state_dir")
	}

	f.Format = bootstrapFormatV0
	if f.Recommendations == nil {
		f.Recommendations = []BootstrapPeerEvidenceV0{}
	}
	if f.Attempts == nil {
		f.Attempts = []BootstrapPeerEvidenceV0{}
	}
	if f.MoreRounds == nil {
		f.MoreRounds = []BootstrapPeerEvidenceV0{}
	}

	data, err := json.MarshalIndent(f, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal bootstrap: %w", err)
	}
	data = append(data, '\n')

	path := bootstrapPath(stateDir)
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return fmt.Errorf("mkdir bootstrap dir: %w", err)
	}
	if err := writeFileAtomic(path, data, 0o600); err != nil {
		return fmt.Errorf("write bootstrap: %w", err)
	}
	return nil
}
