package sessionconfig

import (
	"os"
	"path/filepath"
	"testing"
)

func TestNormalizeLogLevel(t *testing.T) {
	tests := []struct {
		name    string
		in      string
		want    string
		wantErr bool
	}{
		{name: "default", in: "", want: DefaultLogLevel},
		{name: "debug", in: "DEBUG", want: "debug"},
		{name: "invalid", in: "verbose", wantErr: true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := NormalizeLogLevel(tt.in)
			if gotErr := err != nil; gotErr != tt.wantErr {
				t.Fatalf("NormalizeLogLevel(%q) error = %v, want error presence = %t", tt.in, err, tt.wantErr)
			}
			if got != tt.want {
				t.Fatalf("NormalizeLogLevel(%q) = %q, want %q", tt.in, got, tt.want)
			}
		})
	}
}

func TestReadFileDefaultsWhenAbsent(t *testing.T) {
	got, err := ReadFile(filepath.Join(t.TempDir(), "missing.json"))
	if err != nil {
		t.Fatalf("ReadFile(missing) error = %v, want nil", err)
	}
	if got.Preferences.LogLevel != DefaultLogLevel {
		t.Fatalf("ReadFile(missing).Preferences.LogLevel = %q, want %q", got.Preferences.LogLevel, DefaultLogLevel)
	}
}

func TestWriteAndReadFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "data", "session_config.json")
	if err := WriteFile(path, Config{Preferences: Preferences{LogLevel: "trace"}}); err != nil {
		t.Fatalf("WriteFile(%q) error = %v, want nil", path, err)
	}

	got, ok, err := ReadFileIfExists(path)
	if err != nil {
		t.Fatalf("ReadFileIfExists(%q) error = %v, want nil", path, err)
	}
	if !ok {
		t.Fatalf("ReadFileIfExists(%q) exists = false, want true", path)
	}
	if got.Preferences.LogLevel != "trace" {
		t.Fatalf("ReadFileIfExists(%q).Preferences.LogLevel = %q, want trace", path, got.Preferences.LogLevel)
	}
	if info, err := os.Stat(path); err != nil {
		t.Fatalf("os.Stat(%q) error = %v, want nil", path, err)
	} else if info.Mode().Perm() != 0o600 {
		t.Fatalf("os.Stat(%q).Mode = %v, want 0600", path, info.Mode().Perm())
	}
}
