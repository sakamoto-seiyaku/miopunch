package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/miopunch/miopunch/internal/poc"
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
	if !strings.Contains(got, "up --broker <endpoint>") {
		t.Fatalf("run(--help) output missing up --broker help, got:\n%s", got)
	}
	if !strings.Contains(got, "up --log-level trace|debug|info|warn|error") {
		t.Fatalf("run(--help) output missing up --log-level help, got:\n%s", got)
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
	socketPath := filepath.Join(t.TempDir(), "missing-localapi.sock")
	gotExitCode := run([]string{"--localapi", "unix:" + socketPath, "join"}, &out, &out)

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

func TestRun_DaemonNotRunning_JSONFailureEnvelope(t *testing.T) {
	t.Parallel()

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	socketPath := filepath.Join(t.TempDir(), "missing-localapi.sock")
	gotExitCode := run([]string{"--format", "json", "--localapi", "unix:" + socketPath, "join"}, &stdout, &stderr)

	if gotExitCode != 3 {
		t.Fatalf("run(join --format json) exitCode = %d, want %d", gotExitCode, 3)
	}
	if stderr.Len() != 0 {
		t.Fatalf("run(join --format json) stderr = %q, want empty", stderr.String())
	}

	var env poc.EnvelopeJSONV0
	if err := json.Unmarshal(stdout.Bytes(), &env); err != nil {
		t.Fatalf("json.Unmarshal(stdout) error = %v, raw=%s", err, stdout.String())
	}
	if env.Status != "failed" {
		t.Fatalf("env.Status = %q, want %q", env.Status, "failed")
	}
	if env.Stage == "" {
		t.Fatalf("env.Stage = empty, want non-empty")
	}
	if env.ReasonCode != poc.ReasonCodeDaemonNotRunning {
		t.Fatalf("env.ReasonCode = %q, want %q", env.ReasonCode, poc.ReasonCodeDaemonNotRunning)
	}
	if env.ExitCode != poc.ExitCodeUnavailable {
		t.Fatalf("env.ExitCode = %d, want %d", env.ExitCode, poc.ExitCodeUnavailable)
	}
	if len(env.Facts) == 0 {
		t.Fatalf("env.Facts = empty, want non-empty")
	}
	if len(env.Suggestions) == 0 {
		t.Fatalf("env.Suggestions = empty, want non-empty")
	}
}

func TestRun_DaemonNotRunning_ReportExport(t *testing.T) {
	t.Parallel()

	var out bytes.Buffer
	dir := t.TempDir()
	socketPath := filepath.Join(dir, "missing-localapi.sock")
	reportPath := filepath.Join(dir, "failure-report.md")
	gotExitCode := run([]string{"--localapi", "unix:" + socketPath, "--report", reportPath, "join"}, &out, &out)

	if gotExitCode != 3 {
		t.Fatalf("run(join --report) exitCode = %d, want %d", gotExitCode, 3)
	}

	report, err := os.ReadFile(reportPath)
	if err != nil {
		t.Fatalf("os.ReadFile(reportPath) error = %v, want nil", err)
	}
	got := string(report)
	if !strings.Contains(got, "# miopunch task report") {
		t.Fatalf("report missing title, got:\n%s", got)
	}
	if !strings.Contains(got, "- status: `failed`") {
		t.Fatalf("report missing failed status, got:\n%s", got)
	}
	if !strings.Contains(got, "- stage: `cli`") {
		t.Fatalf("report missing stage, got:\n%s", got)
	}
	if !strings.Contains(got, "- reason_code: `DAEMON_NOT_RUNNING`") {
		t.Fatalf("report missing reason_code, got:\n%s", got)
	}
	if !strings.Contains(got, "## Facts") {
		t.Fatalf("report missing facts section, got:\n%s", got)
	}
	if !strings.Contains(got, "## Suggestions") {
		t.Fatalf("report missing suggestions section, got:\n%s", got)
	}
}
