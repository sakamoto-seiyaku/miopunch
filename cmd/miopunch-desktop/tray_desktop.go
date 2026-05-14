//go:build desktop

package main

import "errors"

var errTrayUnsupported = errors.New("desktop tray is unsupported on this platform")

type desktopTray interface {
	Show(onOpen func(), onQuit func()) error
	Close()
}
