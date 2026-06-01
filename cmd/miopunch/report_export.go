package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/miopunch/miopunch/internal/atomicfile"
)

func exportReportMarkdown(path string, report string, redact bool) error {
	path = strings.TrimSpace(path)
	if path == "" || strings.TrimSpace(report) == "" {
		return nil
	}
	if redact {
		report = redactString(report)
	}

	dir := filepath.Dir(path)
	if dir != "." && dir != "" {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return fmt.Errorf("mkdir report dir: %w", err)
		}
	}
	if err := atomicfile.WriteFile(path, []byte(report), 0o600); err != nil {
		return fmt.Errorf("write report: %w", err)
	}
	return nil
}

func failureReportMarkdown(kind string, taskID string, failure failureOutput) string {
	var b strings.Builder
	b.WriteString("# miopunch task report\n\n")
	b.WriteString("- status: `failed`\n")
	if kind = strings.TrimSpace(kind); kind != "" {
		fmt.Fprintf(&b, "- kind: `%s`\n", kind)
	}
	if taskID = strings.TrimSpace(taskID); taskID != "" {
		fmt.Fprintf(&b, "- task_id: `%s`\n", taskID)
	}
	fmt.Fprintf(&b, "- stage: `%s`\n", strings.TrimSpace(failure.Stage))
	fmt.Fprintf(&b, "- reason_code: `%s`\n", failure.ReasonCode)
	fmt.Fprintf(&b, "- exit_code: `%d`\n\n", failure.ExitCode)

	b.WriteString("## Facts\n")
	for _, fact := range failure.Facts {
		msg := strings.TrimSpace(fact.Message)
		if msg == "" {
			continue
		}
		fmt.Fprintf(&b, "- %s\n", msg)
	}

	b.WriteString("\n## Suggestions\n")
	for _, suggestion := range failure.Suggestions {
		msg := strings.TrimSpace(suggestion.Message)
		if msg == "" {
			continue
		}
		fmt.Fprintf(&b, "- %s\n", msg)
	}

	return b.String()
}
