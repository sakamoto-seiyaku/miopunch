//go:build !linux && !windows

package main

import (
	"io"

	"github.com/miopunch/miopunch/internal/poc"
)

func runInstallSystemDaemon(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	_ = args
	return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
		Stage:      "cli",
		ReasonCode: poc.ReasonCodeNotImplemented,
		ExitCode:   poc.ExitCodeBadRequest,
		Facts: []poc.Fact{
			{Message: "install-system-daemon is only supported on linux/windows"},
		},
		Suggestions: []poc.Suggestion{
			{Message: "run: miopunch up (foreground)"},
		},
	})
}

func runUninstallSystemDaemon(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	_ = args
	return exitWithFailure(opt, stdout, stderr, "uninstall-system-daemon", "", failureOutput{
		Stage:      "cli",
		ReasonCode: poc.ReasonCodeNotImplemented,
		ExitCode:   poc.ExitCodeBadRequest,
		Facts: []poc.Fact{
			{Message: "uninstall-system-daemon is only supported on linux/windows"},
		},
		Suggestions: []poc.Suggestion{
			{Message: "run: miopunch up (foreground)"},
		},
	})
}
