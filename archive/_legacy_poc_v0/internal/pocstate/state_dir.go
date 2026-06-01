package pocstate

import (
	"errors"
	"path/filepath"
	"strings"
)

// StateDir derives the POC state directory from the state.json path.
//
// POC v0 rule: state_dir is the directory containing state.json.
func StateDir(statePath string) (string, error) {
	if strings.TrimSpace(statePath) == "" {
		return "", errors.New("empty state path")
	}
	return filepath.Dir(statePath), nil
}
