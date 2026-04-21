package main

import (
	"fmt"
	"io"
	"os"

	"github.com/miopunch/miopunch/internal/poc"
)

func main() {
	os.Exit(run(os.Args[1:], os.Stdout, os.Stderr))
}

func run(args []string, stdout, stderr io.Writer) int {
	opt, rest, err := parseGlobalOptions(args)
	if err != nil {
		return exitWithFailure(opt, stdout, stderr, "", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts: []poc.Fact{
				{Message: err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "run: miopunch --help"},
			},
		})
	}

	if len(rest) == 0 {
		writeFailure(stderr, failureOutput{
			Stage:      "cli",
			ReasonCode: "MISSING_COMMAND",
			ExitCode:   poc.ExitCodeBadRequest,
			Facts: []poc.Fact{
				{Message: "cmd: (missing)"},
			},
			Suggestions: []poc.Suggestion{
				{Message: "run: miopunch --help"},
			},
		})
		fmt.Fprintln(stderr)
		usage(stderr)
		return int(poc.ExitCodeBadRequest)
	}

	cmd := rest[0]
	cmdArgs := rest[1:]

	switch cmd {
	case "-h", "--help", "help":
		usage(stdout)
		return 0
	case "coord", "peer", "stun", "mqtt-broker":
		fmt.Fprintf(stderr, "miopunch %s is a lab/experiment command.\n\n", cmd)
		writeFailure(stderr, failureOutput{
			Stage:      "cli",
			ReasonCode: "LAB_COMMAND_MOVED",
			ExitCode:   poc.ExitCodeBadRequest,
			Facts: []poc.Fact{
				{Message: fmt.Sprintf("cmd: %s", cmd)},
			},
			Suggestions: []poc.Suggestion{
				{Message: fmt.Sprintf("use: miopunch-lab %s [flags]", cmd)},
				{Message: fmt.Sprintf("run: miopunch-lab %s --help", cmd)},
			},
		})
		return int(poc.ExitCodeBadRequest)
	case "up":
		return runUp(cmdArgs, stdout, stderr)
	case "ls":
		return runLS(opt, cmdArgs, stdout, stderr)
	case "invite":
		return runTaskKind(opt, "invite", nil, stdout, stderr)
	case "approve":
		return runTaskKind(opt, "approve", nil, stdout, stderr)
	case "join":
		return runJoin(opt, cmdArgs, stdout, stderr)
	case "ping":
		return runPing(opt, cmdArgs, stdout, stderr)
	case "sh":
		if len(cmdArgs) > 0 && cmdArgs[0] == "ls" {
			return runShLS(opt, cmdArgs[1:], stdout, stderr)
		}
		return runSh(opt, cmdArgs, stdout, stderr)
	case "revoke":
		return runTaskKind(opt, "revoke_member", nil, stdout, stderr)
	case "reset":
		return exitWithFailure(opt, stdout, stderr, "reset", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeNotImplemented,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts: []poc.Fact{
				{Message: "cmd: reset"},
			},
			Suggestions: []poc.Suggestion{
				{Message: "see: docs/roadmap.md (POC roadmap)"},
			},
		})
	case "install-system-daemon":
		return runInstallSystemDaemon(opt, cmdArgs, stdout, stderr)
	case "uninstall-system-daemon":
		return runUninstallSystemDaemon(opt, cmdArgs, stdout, stderr)
	default:
		fmt.Fprintf(stderr, "unknown command: %s\n\n", cmd)
		return exitWithFailure(opt, stdout, stderr, cmd, "", failureOutput{
			Stage:      "cli",
			ReasonCode: "UNKNOWN_COMMAND",
			ExitCode:   poc.ExitCodeBadRequest,
			Facts: []poc.Fact{
				{Message: fmt.Sprintf("cmd: %s", cmd)},
			},
			Suggestions: []poc.Suggestion{
				{Message: "run: miopunch --help"},
			},
		})
	}
}

func usage(w io.Writer) {
	fmt.Fprint(w, `miopunch (POC product CLI)

This binary is reserved for the POC/product CLI (join → ping → sh(tmux)).

For lab/experiments (coord/peer/stun/mqtt-broker), use:
  miopunch-lab <command> [flags]

Usage:
  miopunch [--format human|json] [--localapi <addr>] <command> [args]

Commands (POC, work in progress):
  up
  ls
  invite
  approve
  join
  ping
  sh
  reset
  install-system-daemon
  uninstall-system-daemon

Help:
  miopunch --help
`)
}
