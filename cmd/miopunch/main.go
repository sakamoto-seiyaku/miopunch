package main

import (
	"fmt"
	"io"
	"os"
)

func main() {
	os.Exit(run(os.Args[1:], os.Stdout, os.Stderr))
}

func run(args []string, stdout, stderr io.Writer) int {
	if len(args) == 0 {
		usage(stderr)
		return 2
	}

	switch args[0] {
	case "-h", "--help", "help":
		usage(stdout)
		return 0
	case "coord", "peer", "stun", "mqtt-broker":
		fmt.Fprintf(stderr, "miopunch %s is a lab/experiment command.\n\n", args[0])
		fmt.Fprintf(stderr, "Use `miopunch-lab %s` instead.\n", args[0])
		return 2
	default:
		fmt.Fprintf(stderr, "unknown command: %s\n\n", args[0])
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
