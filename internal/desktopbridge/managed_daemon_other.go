//go:build !windows

package desktopbridge

import "os/exec"

func configureManagedDaemonCmd(*exec.Cmd) {}
