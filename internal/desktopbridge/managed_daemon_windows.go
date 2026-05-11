//go:build windows

package desktopbridge

import (
	"os/exec"
	"syscall"
)

func configureManagedDaemonCmd(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
}
