package bundlepath

import (
	"os"
	"path/filepath"
	"testing"
)

func TestLogPathForExecutable(t *testing.T) {
	dir := t.TempDir()
	exe := filepath.Join(dir, "miopunch")

	got, err := logPathForExecutable(exe, "miopunch.log")
	if err != nil {
		t.Fatalf("logPathForExecutable() error = %v", err)
	}

	want := filepath.Join(dir, "logs", "miopunch.log")
	if got != want {
		t.Fatalf("logPathForExecutable() = %q, want %q", got, want)
	}
	if info, err := os.Stat(filepath.Join(dir, "logs")); err != nil {
		t.Fatalf("logs dir stat error = %v", err)
	} else if !info.IsDir() {
		t.Fatalf("logs path is not a directory")
	}
}

func TestLogPathForExecutableRejectsEmptyInputs(t *testing.T) {
	if _, err := logPathForExecutable("", "miopunch.log"); err == nil {
		t.Fatalf("logPathForExecutable(empty exe) error = nil, want error")
	}
	if _, err := logPathForExecutable("/tmp/miopunch", " "); err == nil {
		t.Fatalf("logPathForExecutable(empty name) error = nil, want error")
	}
}

func TestStatePathForExecutable(t *testing.T) {
	dir := t.TempDir()
	exe := filepath.Join(dir, "miopunch")

	got, err := StatePathForExecutable(exe)
	if err != nil {
		t.Fatalf("StatePathForExecutable() error = %v", err)
	}

	want := filepath.Join(dir, "data", "state.json")
	if got != want {
		t.Fatalf("StatePathForExecutable() = %q, want %q", got, want)
	}
	if info, err := os.Stat(filepath.Join(dir, "data")); err != nil {
		t.Fatalf("data dir stat error = %v", err)
	} else if !info.IsDir() {
		t.Fatalf("data path is not a directory")
	}
}

func TestStatePathForExecutableRejectsEmptyInput(t *testing.T) {
	if _, err := StatePathForExecutable(""); err == nil {
		t.Fatalf("StatePathForExecutable(empty exe) error = nil, want error")
	}
}

func TestSessionConfigPathForExecutable(t *testing.T) {
	dir := t.TempDir()
	exe := filepath.Join(dir, "miopunch")

	got, err := SessionConfigPathForExecutable(exe)
	if err != nil {
		t.Fatalf("SessionConfigPathForExecutable() error = %v", err)
	}

	want := filepath.Join(dir, "data", "session_config.json")
	if got != want {
		t.Fatalf("SessionConfigPathForExecutable() = %q, want %q", got, want)
	}
	if info, err := os.Stat(filepath.Join(dir, "data")); err != nil {
		t.Fatalf("data dir stat error = %v", err)
	} else if !info.IsDir() {
		t.Fatalf("data path is not a directory")
	}
}

func TestSessionConfigPathForExecutableRejectsEmptyInput(t *testing.T) {
	if _, err := SessionConfigPathForExecutable(""); err == nil {
		t.Fatalf("SessionConfigPathForExecutable(empty exe) error = nil, want error")
	}
}

func TestLocalAPIPathForExecutable(t *testing.T) {
	dir := t.TempDir()
	exe := filepath.Join(dir, "miopunch")

	got, err := LocalAPIPathForExecutable(exe)
	if err != nil {
		t.Fatalf("LocalAPIPathForExecutable() error = %v", err)
	}

	want := filepath.Join(dir, "data", "localapi.sock")
	if got != want {
		t.Fatalf("LocalAPIPathForExecutable() = %q, want %q", got, want)
	}
	if info, err := os.Stat(filepath.Join(dir, "data")); err != nil {
		t.Fatalf("data dir stat error = %v", err)
	} else if !info.IsDir() {
		t.Fatalf("data path is not a directory")
	}
}

func TestLocalAPIPathForExecutableRejectsEmptyInput(t *testing.T) {
	if _, err := LocalAPIPathForExecutable(""); err == nil {
		t.Fatalf("LocalAPIPathForExecutable(empty exe) error = nil, want error")
	}
}
