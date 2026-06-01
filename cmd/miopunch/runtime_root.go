package main

import (
	"fmt"
	"os"
	"os/user"
	"path/filepath"
	"strings"
)

func resolveRuntimeRoot(statePath string) (string, error) {
	return resolveRuntimeRootWithUserHome(statePath, os.UserHomeDir, currentUserHomeDir)
}

func resolveRuntimeRootWithUserHome(
	statePath string,
	userHomeDir func() (string, error),
	currentUserHomeDir func() (string, error),
) (string, error) {
	if trimmed := strings.TrimSpace(statePath); trimmed != "" {
		return filepath.Clean(filepath.Dir(trimmed)), nil
	}

	if stateHome := strings.TrimSpace(os.Getenv("XDG_STATE_HOME")); stateHome != "" {
		return filepath.Join(stateHome, "miopunch", "pocv1"), nil
	}

	home, err := userHomeDir()
	if err == nil && strings.TrimSpace(home) != "" {
		return filepath.Join(home, ".local", "state", "miopunch", "pocv1"), nil
	}

	home, currentErr := currentUserHomeDir()
	if currentErr == nil && strings.TrimSpace(home) != "" {
		return filepath.Join(home, ".local", "state", "miopunch", "pocv1"), nil
	}
	if currentErr != nil {
		return "", fmt.Errorf("resolve user home: %w (fallback lookup: %v)", err, currentErr)
	}
	return "", fmt.Errorf("resolve user home: %w", err)
}

func currentUserHomeDir() (string, error) {
	current, err := user.Current()
	if err != nil {
		return "", err
	}
	if strings.TrimSpace(current.HomeDir) == "" {
		return "", fmt.Errorf("current user home directory is empty")
	}
	return current.HomeDir, nil
}
