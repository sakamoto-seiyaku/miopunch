package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"strings"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
)

func exitWithFailure(
	opt globalOptions,
	stdout io.Writer,
	stderr io.Writer,
	kind string,
	taskID string,
	failure failureOutput,
) int {
	facts := append([]poc.Fact(nil), failure.Facts...)
	suggestions := append([]poc.Suggestion(nil), failure.Suggestions...)
	if opt.Redact {
		facts = redactFacts(facts)
		suggestions = redactSuggestions(suggestions)
	}
	reportFailure := failureOutput{
		Stage:       failure.Stage,
		ReasonCode:  failure.ReasonCode,
		ExitCode:    failure.ExitCode,
		Facts:       facts,
		Suggestions: suggestions,
	}
	if err := exportReportMarkdown(opt.ReportPath, failureReportMarkdown(kind, taskID, reportFailure), false); err != nil {
		fmt.Fprintf(stderr, "warning: report export failed: %v\n", err)
	}

	if opt.Format == outputFormatJSON {
		env := poc.NewEnvelopeJSONV0()
		env.TaskID = strings.TrimSpace(taskID)
		env.Kind = strings.TrimSpace(kind)
		env.Status = "failed"
		env.Stage = strings.TrimSpace(failure.Stage)
		env.ReasonCode = failure.ReasonCode
		env.ExitCode = failure.ExitCode
		env.Facts = facts
		env.Suggestions = suggestions
		writeEnvelopeJSON(stdout, env)
		return int(failure.ExitCode)
	}

	writeFailure(stderr, reportFailure)
	return int(failure.ExitCode)
}

func exitWithError(
	opt globalOptions,
	stdout io.Writer,
	stderr io.Writer,
	kind string,
	taskID string,
	err error,
) int {
	if err == nil {
		return 0
	}

	var connErr *localAPIConnectionError
	if errors.As(err, &connErr) {
		return exitWithFailure(opt, stdout, stderr, kind, taskID, connErr.Failure)
	}

	var apiErr *localapi.APIError
	if errors.As(err, &apiErr) {
		return exitWithFailure(opt, stdout, stderr, kind, taskID, failureOutput{
			Stage:       apiErr.Response.Stage,
			ReasonCode:  apiErr.Response.ReasonCode,
			ExitCode:    apiErr.Response.ExitCode,
			Facts:       apiErr.Response.Facts,
			Suggestions: apiErr.Response.Suggestions,
		})
	}

	if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
		return exitWithFailure(opt, stdout, stderr, kind, taskID, failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeTimeout,
			ExitCode:   poc.ExitCodeTimeout,
			Facts: []poc.Fact{
				{Message: "error=" + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
	}

	return exitWithFailure(opt, stdout, stderr, kind, taskID, failureOutput{
		Stage:      "cli",
		ReasonCode: poc.ReasonCodeInternal,
		ExitCode:   poc.ExitCodeInternal,
		Facts: []poc.Fact{
			{Message: "error=" + err.Error()},
		},
		Suggestions: []poc.Suggestion{
			{Message: "retry"},
		},
	})
}

func exitWithActionSuccess(
	opt globalOptions,
	stdout io.Writer,
	stderr io.Writer,
	kind string,
	result pocruntime.ActionResult,
) int {
	if err := exportReportMarkdown(opt.ReportPath, result.ReportMarkdown, opt.Redact); err != nil {
		return exitWithFailure(opt, stdout, stderr, kind, result.ShellSessionID, failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "export report: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "check --report path and retry"},
			},
		})
	}

	facts := append([]poc.Fact(nil), result.Evidence.Facts...)
	suggestions := append([]poc.Suggestion(nil), result.Evidence.Suggestions...)
	if opt.Redact {
		facts = redactFacts(facts)
		suggestions = redactSuggestions(suggestions)
	}

	if opt.Format == outputFormatJSON {
		env := poc.NewEnvelopeJSONV0()
		env.TaskID = strings.TrimSpace(result.ShellSessionID)
		env.Kind = strings.TrimSpace(kind)
		env.Status = "done"
		env.Stage = string(result.Stage)
		env.ReasonCode = result.ReasonCode
		env.ExitCode = result.ExitCode
		env.Facts = facts
		if suggestions != nil {
			env.Suggestions = suggestions
		}
		writeEnvelopeJSON(stdout, env)
		return int(result.ExitCode)
	}

	lines := append([]string(nil), result.Lines...)
	if len(lines) == 0 && strings.TrimSpace(result.Summary.Text) != "" {
		lines = append(lines, strings.TrimSpace(result.Summary.Text))
	}
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		fmt.Fprintln(stdout, line)
	}
	return int(result.ExitCode)
}
