//go:build windows

package pocstate

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
)

func DefaultStatePath() (string, error) {
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
