//go:build desktop && !windows

package main

import (
	"fmt"
	"os"
)

func reportStartupError(err error) {
	if err == nil {
		return
	}
	fmt.Fprintln(os.Stderr, "miopunch-desktop failed to start:", err)
}
