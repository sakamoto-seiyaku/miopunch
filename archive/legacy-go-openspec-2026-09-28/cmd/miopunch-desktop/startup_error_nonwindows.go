//go:build desktop && !windows

package main

import (
	"fmt"
	"os"
	"strings"
)

func reportStartupError(err error) {
	if err == nil {
		return
	}

	msg := "miopunch-desktop failed to start: " + err.Error()
	if isLinuxGTKStartupError(err) {
		msg += "\n\nLinux GUI startup checks:" +
			"\n  - Run from a local graphical desktop session, not a headless SSH shell." +
			"\n  - Check: echo \"$DISPLAY $WAYLAND_DISPLAY\"" +
			"\n  - Check missing shared libraries: ldd ./miopunch-desktop | grep 'not found'" +
			"\n  - Install your distro GTK/WebKitGTK runtime packages if ldd reports missing libraries." +
			"\n  - Runtime log: ./logs/miopunch-desktop.log"
	}
	fmt.Fprintln(os.Stderr, msg)
}

func isLinuxGTKStartupError(err error) bool {
	if err == nil {
		return false
	}
	msg := err.Error()
	return strings.Contains(msg, "failed to init GTK") ||
		strings.Contains(msg, "no Linux desktop display found")
}
