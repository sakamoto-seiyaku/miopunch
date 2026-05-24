//go:build !windows

package pocstate

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
)

func DefaultStatePath() (string, error) {
	if runtime.GOOS == "linux" && os.Geteuid() == 0 {
		// POC v0 system state dir (best effort; aligns with docs/notes defaults).
		return filepath.Join("/var/lib/miopunch", "state.json"), nil
	}
	return UserStatePath()
}

func UserStatePath() (string, error) {
	dir, err := os.UserConfigDir()
	if err != nil {
		return "", fmt.Errorf("user config dir: %w", err)
	}
	if dir == "" {
		return "", errors.New("user config dir is empty")
	}
	return filepath.Join(dir, "miopunch", "state.json"), nil
}
