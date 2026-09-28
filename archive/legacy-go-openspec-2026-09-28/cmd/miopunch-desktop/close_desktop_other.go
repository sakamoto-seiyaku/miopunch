//go:build desktop && !linux

package main

import "context"

func (a *App) beforeCloseLinux(context.Context) bool {
	return false
}
