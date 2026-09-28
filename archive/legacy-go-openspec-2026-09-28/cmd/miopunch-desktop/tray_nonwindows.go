//go:build desktop && !windows

package main

type noopTray struct{}

func newPlatformTray() desktopTray {
	return noopTray{}
}

func (noopTray) Show(func(), func()) error {
	return errTrayUnsupported
}

func (noopTray) Close() {}
