//go:build desktop && !windows && !linux

package main

import "github.com/wailsapp/wails/v2/pkg/options"

func configurePlatformOptions(appOptions *options.App, _ *App) {
	appOptions.HideWindowOnClose = false
}
