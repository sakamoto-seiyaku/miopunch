//go:build !windows

package main

import "os"

func isRootOperator() bool {
	return os.Geteuid() == 0
}
