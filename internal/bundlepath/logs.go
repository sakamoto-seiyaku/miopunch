// Package bundlepath resolves paths relative to a portable session bundle.
package bundlepath

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// LogPath returns a path under the current executable directory's logs folder.
func LogPath(fileName string) (string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", fmt.Errorf("resolve executable: %w", err)
	}
	return logPathForExecutable(exe, fileName)
}

// StatePath returns the portable session state path for the current executable.
func StatePath() (string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", fmt.Errorf("resolve executable: %w", err)
	}
	return StatePathForExecutable(exe)
}

// StatePathForExecutable returns the portable session state path for exePath.
func StatePathForExecutable(exePath string) (string, error) {
	return dataPathForExecutable(exePath, "state.json")
}

func logPathForExecutable(exePath string, fileName string) (string, error) {
	if strings.TrimSpace(exePath) == "" {
		return "", fmt.Errorf("empty executable path")
	}
	fileName = strings.TrimSpace(fileName)
	if fileName == "" {
		return "", fmt.Errorf("empty log file name")
	}

	logDir := filepath.Join(filepath.Dir(exePath), "logs")
	if err := os.MkdirAll(logDir, 0o755); err != nil {
		return "", fmt.Errorf("create logs dir: %w", err)
	}
	return filepath.Join(logDir, fileName), nil
}

func dataPathForExecutable(exePath string, fileName string) (string, error) {
	if strings.TrimSpace(exePath) == "" {
		return "", fmt.Errorf("empty executable path")
	}
	fileName = strings.TrimSpace(fileName)
	if fileName == "" {
		return "", fmt.Errorf("empty data file name")
	}

	dataDir := filepath.Join(filepath.Dir(exePath), "data")
	if err := os.MkdirAll(dataDir, 0o700); err != nil {
		return "", fmt.Errorf("create data dir: %w", err)
	}
	return filepath.Join(dataDir, fileName), nil
}
