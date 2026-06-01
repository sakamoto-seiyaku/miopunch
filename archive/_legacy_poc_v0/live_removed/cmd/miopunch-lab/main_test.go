package main

import (
	"bytes"
	"strings"
	"testing"
)

func TestRun_CoordHelp(t *testing.T) {
	t.Parallel()

	var out bytes.Buffer
	gotExitCode := run([]string{"coord", "--help"}, &out, &out)

	if gotExitCode != 0 {
		t.Fatalf("run(coord --help) exitCode = %d, want %d", gotExitCode, 0)
	}

	got := out.String()
	if !strings.Contains(got, "Usage of coord") {
		t.Fatalf("run(coord --help) output missing usage, got:\n%s", got)
	}
	if !strings.Contains(got, "control plane protocol") {
		t.Fatalf("run(coord --help) output missing flag help, got:\n%s", got)
	}
}
