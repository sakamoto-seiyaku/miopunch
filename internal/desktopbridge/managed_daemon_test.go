package desktopbridge

import (
	"path/filepath"
	"reflect"
	"testing"
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
