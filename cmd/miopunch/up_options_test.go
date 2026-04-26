package main

import "testing"

func TestParseUpOptions_LabOverrides(t *testing.T) {
	got, rest, err := parseUpOptions([]string{
		"--localapi", "unix:/tmp/miopunch-lab.sock",
		"--state_path", "/tmp/miopunch/state.json",
	})
	if err != nil {
		t.Fatalf("parseUpOptions(lab overrides) error = %v", err)
	}
	if len(rest) != 0 {
		t.Fatalf("parseUpOptions(lab overrides) rest = %v, want empty", rest)
	}
	if got.LocalAPIOverride != "unix:/tmp/miopunch-lab.sock" {
		t.Errorf("parseUpOptions(lab overrides).LocalAPIOverride = %q, want %q", got.LocalAPIOverride, "unix:/tmp/miopunch-lab.sock")
	}
	if got.StatePath != "/tmp/miopunch/state.json" {
		t.Errorf("parseUpOptions(lab overrides).StatePath = %q, want %q", got.StatePath, "/tmp/miopunch/state.json")
	}
}

func TestParseUpOptions_InvalidLocalAPI(t *testing.T) {
	_, _, err := parseUpOptions([]string{"--localapi", "/tmp/missing-prefix.sock"})
	if err == nil {
		t.Fatal("parseUpOptions(invalid localapi) error = nil, want error")
	}
}
