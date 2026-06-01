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
		return runUp(opt, cmdArgs, stdout, stderr)
	case "ls":
		return runLS(opt, cmdArgs, stdout, stderr)
	case "init-network":
		return runInitNetwork(opt, cmdArgs, stdout, stderr)
	case "invite":
		return runInvite(opt, cmdArgs, stdout, stderr)
	case "approve":
		return runApprove(opt, cmdArgs, stdout, stderr)
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
		return runRevoke(opt, cmdArgs, stdout, stderr)
	case "debug-conpty-smoke":
		return runDebugConPTYSmoke(opt, cmdArgs, stdout, stderr)
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
  miopunch [--format human|json] [--localapi <addr>] [--report <path>] [--redact] <command> [args]

Global flags:
  --format human|json   Output format for CLI (default: human)
  --localapi <addr>     LocalAPI address (unix socket / named pipe)
  --report <path>       Export this command's task report as Markdown
  --redact              Redact secrets in CLI output and --report export:
                        invite_code, secret_key, net_secret_b64, invite_secret_b64

Commands (POC, work in progress):
  up
  ls
  init-network
  invite
  approve
  join
  ping
  sh
  revoke
  install-system-daemon
  uninstall-system-daemon

Command flags:
  up --http_panel                    Enable loopback-only HTTP panel UI (MD3)
  up --http_panel_listen_addr <addr> HTTP panel listen address (default: 127.0.0.1:27400; host must be 127.0.0.1)
  up --broker <endpoint>            Use an explicit MQTT broker endpoint for init-network
  up --log-level trace|debug|info|warn|error
                                    Set daemon log level (default: info)
  up --session                       Use portable session mode with ./data/state.json by default
  up --state_path <path>             Override daemon state path (lab/testing)

Help:
  miopunch --help
`)
}
