//go:build !windows

package main

import (
	"context"
	"os"
	"os/signal"
	"syscall"

	"golang.org/x/term"
)

func watchResize(ctx context.Context, stdinFD int, send func(cols, rows int)) {
	ch := make(chan os.Signal, 1)
	signal.Notify(ch, syscall.SIGWINCH)

	go func() {
		defer signal.Stop(ch)
		for {
			select {
			case <-ctx.Done():
				return
			case <-ch:
				cols, rows, err := term.GetSize(stdinFD)
				if err != nil {
					continue
				}
				send(cols, rows)
			}
		}
	}()
}
