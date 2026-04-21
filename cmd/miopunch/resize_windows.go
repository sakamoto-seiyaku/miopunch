//go:build windows

package main

import "context"

func watchResize(ctx context.Context, stdinFD int, send func(cols, rows int)) {
	_ = ctx
	_ = stdinFD
	_ = send
}
