//go:build windows

package main

import "os/exec"

func configureBootstrapCommand(cmd *exec.Cmd) {
	_ = cmd
}
