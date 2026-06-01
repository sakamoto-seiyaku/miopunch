package task

import (
	"fmt"
	"strings"
	"time"
)

func buildReportMarkdown(t Task) string {
	var b strings.Builder

	fmt.Fprintln(&b, "# Task Report")
	fmt.Fprintln(&b)

	fmt.Fprintln(&b, "## Summary")
	fmt.Fprintln(&b)
	fmt.Fprintf(&b, "- task_id: %s\n", t.ID)
	fmt.Fprintf(&b, "- kind: %s\n", t.Kind)
	fmt.Fprintf(&b, "- status: %s\n", t.Status)
	fmt.Fprintf(&b, "- stage: %s\n", t.Stage)
	if t.ReasonCode != "" {
		fmt.Fprintf(&b, "- reason_code: %s\n", t.ReasonCode)
	}
	if t.ExitCode != 0 {
		fmt.Fprintf(&b, "- exit_code: %d\n", t.ExitCode)
	}
	fmt.Fprintln(&b)

	fmt.Fprintln(&b, "## Timeline")
	fmt.Fprintln(&b)
	if len(t.Timeline) == 0 {
		fmt.Fprintln(&b, "- (no timeline entries)")
	} else {
		for _, e := range t.Timeline {
			msg := strings.TrimSpace(e.Message)
			if msg == "" {
				msg = "-"
			}
			fmt.Fprintf(&b, "- %s %s: %s\n", e.At.Format(time.RFC3339), e.Stage, msg)
		}
	}
	fmt.Fprintln(&b)

	fmt.Fprintln(&b, "## Facts")
	fmt.Fprintln(&b)
	if len(t.Facts) == 0 {
		fmt.Fprintln(&b, "- (none)")
	} else {
		for _, fact := range t.Facts {
			fmt.Fprintf(&b, "- %s\n", strings.TrimSpace(fact.Message))
		}
	}
	fmt.Fprintln(&b)

	fmt.Fprintln(&b, "## Suggestions")
	fmt.Fprintln(&b)
	if len(t.Suggestions) == 0 {
		fmt.Fprintln(&b, "- (none)")
	} else {
		for _, s := range t.Suggestions {
			fmt.Fprintf(&b, "- %s\n", strings.TrimSpace(s.Message))
		}
	}
	fmt.Fprintln(&b)

	return b.String()
}
