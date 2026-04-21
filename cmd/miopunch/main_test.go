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
}

func TestRun_LabCommandMoved(t *testing.T) {
	t.Parallel()

	var out bytes.Buffer
	gotExitCode := run([]string{"coord"}, &out, &out)

	if gotExitCode != 2 {
		t.Fatalf("run(coord) exitCode = %d, want %d", gotExitCode, 2)
	}

	got := out.String()
	if !strings.Contains(got, "miopunch-lab coord") {
		t.Fatalf("run(coord) output missing guidance, got:\n%s", got)
	}
}
