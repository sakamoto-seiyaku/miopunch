//go:build desktop && linux

package main

import "context"

func (a *App) beforeCloseLinux(context.Context) bool {
	a.markQuitRequested()
	go a.exitNow(0)
	return true
}
