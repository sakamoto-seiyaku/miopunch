package main

import "testing"

func TestParseUpOptions_LabOverrides(t *testing.T) {
	got, rest, err := parseUpOptions([]string{
		"--localapi", "unix:/tmp/miopunch-lab.sock",
		"--broker", "mqtt://broker.example:1883",
		"--log-level", "debug",
		"--session",
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
	if got.BrokerOverride != "mqtt://broker.example:1883" {
		t.Errorf("parseUpOptions(lab overrides).BrokerOverride = %q, want %q", got.BrokerOverride, "mqtt://broker.example:1883")
	}
	if got.LogLevel != "debug" {
		t.Errorf("parseUpOptions(lab overrides).LogLevel = %q, want %q", got.LogLevel, "debug")
	}
	if got.StatePath != "/tmp/miopunch/state.json" {
		t.Errorf("parseUpOptions(lab overrides).StatePath = %q, want %q", got.StatePath, "/tmp/miopunch/state.json")
	}
	if !got.Session {
		t.Errorf("parseUpOptions(lab overrides).Session = false, want true")
	}
}

func TestParseUpOptions_InvalidLocalAPI(t *testing.T) {
	_, _, err := parseUpOptions([]string{"--localapi", "/tmp/missing-prefix.sock"})
	if err == nil {
		t.Fatal("parseUpOptions(invalid localapi) error = nil, want error")
	}
}

func TestParseUpOptions_BrokerEquals(t *testing.T) {
	got, rest, err := parseUpOptions([]string{"--broker=tcp://broker.example:1883"})
	if err != nil {
		t.Fatalf("parseUpOptions(broker equals) error = %v", err)
	}
	if len(rest) != 0 {
		t.Fatalf("parseUpOptions(broker equals) rest = %v, want empty", rest)
	}
	if got.BrokerOverride != "tcp://broker.example:1883" {
		t.Errorf("parseUpOptions(broker equals).BrokerOverride = %q, want %q", got.BrokerOverride, "tcp://broker.example:1883")
	}
}

func TestParseUpOptions_InvalidLogLevel(t *testing.T) {
	_, _, err := parseUpOptions([]string{"--log-level", "verbose"})
	if err == nil {
		t.Fatal("parseUpOptions(invalid log-level) error = nil, want error")
	}
}
