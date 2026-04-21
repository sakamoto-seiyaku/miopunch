package main

import (
	"bytes"
	"strings"
	"testing"
)

func TestRun_Help(t *testing.T) {
	t.Parallel()

	var out bytes.Buffer
	gotExitCode := run([]string{"--help"}, &out, &out)

	if gotExitCode != 0 {
		t.Fatalf("run(--help) exitCode = %d, want %d", gotExitCode, 0)
	}

	got := out.String()
	if !strings.Contains(got, "miopunch (POC product CLI)") {
		t.Fatalf("run(--help) output missing header, got:\n%s", got)
	}
	if !strings.Contains(got, "miopunch-lab") {
		t.Fatalf("run(--help) output missing miopunch-lab hint, got:\n%s", got)
	}
	if !strings.Contains(got, "install-system-daemon") {
		t.Fatalf("run(--help) output missing install-system-daemon, got:\n%s", got)
	}
	if !strings.Contains(got, "--format human|json") {
		t.Fatalf("run(--help) output missing global flags, got:\n%s", got)
	}
}

func TestRun_LabCommandMoved(t *testing.T) {
	t.Parallel()

	var out bytes.Buffer
	gotExitCode := run([]string{"coord"}, &out, &out)

	if gotExitCode != 2 {
		t.Fatalf("run(coord) exitCode = %d, want %d", gotExitCode, 2)
	}

	got := out.String()
	if !strings.Contains(got, "stage=") {
		t.Fatalf("run(coord) output missing stage, got:\n%s", got)
	}
	if !strings.Contains(got, "reason_code=LAB_COMMAND_MOVED") {
		t.Fatalf("run(coord) output missing reason_code, got:\n%s", got)
	}
	if !strings.Contains(got, "facts:") {
		t.Fatalf("run(coord) output missing facts, got:\n%s", got)
	}
	if !strings.Contains(got, "suggestions:") {
		t.Fatalf("run(coord) output missing suggestions, got:\n%s", got)
	}
	if !strings.Contains(got, "miopunch-lab coord") {
		t.Fatalf("run(coord) output missing guidance, got:\n%s", got)
	}
}

func TestRun_DaemonNotRunning_HasFailureEnvelope(t *testing.T) {
	t.Parallel()

	var out bytes.Buffer
	gotExitCode := run([]string{"join"}, &out, &out)

	if gotExitCode != 3 {
		t.Fatalf("run(join) exitCode = %d, want %d", gotExitCode, 3)
	}

	got := out.String()
	if !strings.Contains(got, "stage=") {
		t.Fatalf("run(join) output missing stage, got:\n%s", got)
	}
	if !strings.Contains(got, "reason_code=DAEMON_NOT_RUNNING") {
		t.Fatalf("run(join) output missing reason_code, got:\n%s", got)
	}
	if !strings.Contains(got, "exit_code=3") {
		t.Fatalf("run(join) output missing exit_code, got:\n%s", got)
	}
	if !strings.Contains(got, "facts:") {
		t.Fatalf("run(join) output missing facts, got:\n%s", got)
	}
	if !strings.Contains(got, "suggestions:") {
		t.Fatalf("run(join) output missing suggestions, got:\n%s", got)
	}
	if !strings.Contains(got, "miopunch up") {
		t.Fatalf("run(join) output missing guidance, got:\n%s", got)
	}
}
