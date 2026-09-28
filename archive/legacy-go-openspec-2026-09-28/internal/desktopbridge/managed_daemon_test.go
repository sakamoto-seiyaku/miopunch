package desktopbridge

import (
	"path/filepath"
	"reflect"
	"testing"

	"github.com/miopunch/miopunch/internal/bundlepath"
	"github.com/miopunch/miopunch/internal/sessionconfig"
)

func TestManagedDaemonArgsUsesSessionMode(t *testing.T) {
	dir := t.TempDir()
	daemonPath := filepath.Join(dir, "miopunch")
	want := []string{"up", "--session", "--state_path", filepath.Join(dir, "data", "state.json")}
	got, err := managedDaemonArgs(daemonPath)
	if err != nil {
		t.Fatalf("managedDaemonArgs(%q) error = %v", daemonPath, err)
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("managedDaemonArgs(%q) = %v, want %v", daemonPath, got, want)
	}
}

func TestManagedDaemonArgsUsesSessionLogLevel(t *testing.T) {
	dir := t.TempDir()
	daemonPath := filepath.Join(dir, "miopunch")
	configPath, err := bundlepath.SessionConfigPathForExecutable(daemonPath)
	if err != nil {
		t.Fatalf("SessionConfigPathForExecutable(%q) error = %v, want nil", daemonPath, err)
	}
	if err := sessionconfig.WriteFile(configPath, sessionconfig.Config{
		Preferences: sessionconfig.Preferences{LogLevel: "debug"},
	}); err != nil {
		t.Fatalf("sessionconfig.WriteFile(%q) error = %v, want nil", configPath, err)
	}

	want := []string{
		"up",
		"--session",
		"--state_path",
		filepath.Join(dir, "data", "state.json"),
		"--log-level",
		"debug",
	}
	got, err := managedDaemonArgs(daemonPath)
	if err != nil {
		t.Fatalf("managedDaemonArgs(%q) error = %v, want nil", daemonPath, err)
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("managedDaemonArgs(%q) = %v, want %v", daemonPath, got, want)
	}
}
