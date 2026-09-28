//go:build desktop && windows

package main

import "github.com/wailsapp/wails/v2/pkg/options"

func configurePlatformOptions(appOptions *options.App, a *App) {
	appOptions.HideWindowOnClose = false
	appOptions.OnBeforeClose = a.beforeClose
}
