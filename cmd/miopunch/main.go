package main

import (
	"fmt"
	"io"
	"os"
)

func main() {
	os.Exit(run(os.Args[1:], os.Stdout, os.Stderr))
}

type failureOutput struct {
	Stage       string
	ReasonCode  string
	Facts       []string
	Suggestions []string
}

func (f failureOutput) write(w io.Writer) {
	fmt.Fprintf(w, "stage=%s\n", f.Stage)
	fmt.Fprintf(w, "reason_code=%s\n", f.ReasonCode)
	fmt.Fprintln(w, "facts:")
	for _, fact := range f.Facts {
		fmt.Fprintf(w, "- %s\n", fact)
	}
	fmt.Fprintln(w, "suggestions:")
	for _, suggestion := range f.Suggestions {
		fmt.Fprintf(w, "- %s\n", suggestion)
	}
}

func run(args []string, stdout, stderr io.Writer) int {
	if len(args) == 0 {
		failureOutput{
			Stage:      "cli",
			ReasonCode: "MISSING_COMMAND",
			Facts: []string{
				"cmd: (missing)",
			},
			Suggestions: []string{
				"run: miopunch --help",
			},
		}.write(stderr)
		fmt.Fprintln(stderr)
		usage(stderr)
		return 2
	}

	switch args[0] {
	case "-h", "--help", "help":
		usage(stdout)
		return 0
	case "coord", "peer", "stun", "mqtt-broker":
		fmt.Fprintf(stderr, "miopunch %s is a lab/experiment command.\n\n", args[0])
		failureOutput{
			Stage:      "cli",
			ReasonCode: "LAB_COMMAND_MOVED",
			Facts: []string{
				fmt.Sprintf("cmd: %s", args[0]),
			},
			Suggestions: []string{
				fmt.Sprintf("use: miopunch-lab %s [flags]", args[0]),
				fmt.Sprintf("run: miopunch-lab %s --help", args[0]),
			},
		}.write(stderr)
		return 2
	case "up", "ls", "invite", "approve", "join", "ping", "sh", "reset":
		failureOutput{
			Stage:      "cli",
			ReasonCode: "NOT_IMPLEMENTED",
			Facts: []string{
				fmt.Sprintf("cmd: %s", args[0]),
			},
			Suggestions: []string{
				"run: miopunch --help",
				"see: docs/roadmap.md (POC roadmap)",
			},
		}.write(stderr)
		return 2
	default:
		fmt.Fprintf(stderr, "unknown command: %s\n\n", args[0])
		failureOutput{
			Stage:      "cli",
			ReasonCode: "UNKNOWN_COMMAND",
			Facts: []string{
				fmt.Sprintf("cmd: %s", args[0]),
			},
			Suggestions: []string{
				"run: miopunch --help",
			},
		}.write(stderr)
		fmt.Fprintln(stderr)
		usage(stderr)
		return 2
	}
}

func usage(w io.Writer) {
	fmt.Fprint(w, `miopunch (POC product CLI)

This binary is reserved for the POC/product CLI (join → ping → sh(tmux)).

For lab/experiments (coord/peer/stun/mqtt-broker), use:
  miopunch-lab <command> [flags]

Usage:
  miopunch <command> [args]

Commands (POC, work in progress):
  up
  ls
  invite
  approve
  join
  ping
  sh
  reset

Help:
  miopunch --help
`)
}
