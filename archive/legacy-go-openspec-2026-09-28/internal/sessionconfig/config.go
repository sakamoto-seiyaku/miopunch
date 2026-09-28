package sessionconfig

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/miopunch/miopunch/internal/atomicfile"
)

const (
	// DefaultLogLevel is the log level used when no session preference exists.
	DefaultLogLevel = "info"
)

var validLogLevels = map[string]struct{}{
	"trace": {},
	"debug": {},
	"info":  {},
	"warn":  {},
	"error": {},
}

// Config is the persisted portable session configuration.
type Config struct {
	Preferences Preferences `json:"preferences,omitempty"`
}

// Preferences contains user-facing portable session preferences.
type Preferences struct {
	LogLevel string `json:"log_level,omitempty"`
}

// Default returns a config populated with safe default values.
func Default() Config {
	return Config{
		Preferences: Preferences{
			LogLevel: DefaultLogLevel,
		},
	}
}

// NormalizeLogLevel validates and canonicalizes a log level string.
func NormalizeLogLevel(value string) (string, error) {
	level := strings.ToLower(strings.TrimSpace(value))
	if level == "" {
		return DefaultLogLevel, nil
	}
	if _, ok := validLogLevels[level]; !ok {
		return "", fmt.Errorf("invalid log level %q", value)
	}
	return level, nil
}

// ReadFile reads a session config, returning defaults when the file is absent.
func ReadFile(path string) (Config, error) {
	config, ok, err := ReadFileIfExists(path)
	if err != nil {
		return Config{}, err
	}
	if !ok {
		return Default(), nil
	}
	return config, nil
}

// ReadFileIfExists reads a session config and reports whether it existed.
func ReadFileIfExists(path string) (Config, bool, error) {
	path = strings.TrimSpace(path)
	if path == "" {
		return Config{}, false, errors.New("empty session config path")
	}

	data, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return Config{}, false, nil
	}
	if err != nil {
		return Config{}, false, fmt.Errorf("read session config: %w", err)
	}

	config := Default()
	if err := json.Unmarshal(data, &config); err != nil {
		return Config{}, true, fmt.Errorf("decode session config: %w", err)
	}
	level, err := NormalizeLogLevel(config.Preferences.LogLevel)
	if err != nil {
		return Config{}, true, err
	}
	config.Preferences.LogLevel = level
	return config, true, nil
}

// WriteFile writes a session config atomically.
func WriteFile(path string, config Config) error {
	path = strings.TrimSpace(path)
	if path == "" {
		return errors.New("empty session config path")
	}

	level, err := NormalizeLogLevel(config.Preferences.LogLevel)
	if err != nil {
		return err
	}
	config.Preferences.LogLevel = level

	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return fmt.Errorf("create session config dir: %w", err)
	}
	data, err := json.MarshalIndent(config, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal session config: %w", err)
	}
	data = append(data, '\n')
	if err := atomicfile.WriteFile(path, data, 0o600); err != nil {
		return fmt.Errorf("write session config: %w", err)
	}
	return nil
}
