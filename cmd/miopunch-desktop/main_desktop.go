//go:build desktop

package main

import (
	"embed"
	"fmt"
	"os"
	"runtime/debug"

	"github.com/wailsapp/wails/v2"
	"github.com/wailsapp/wails/v2/pkg/options"
	"github.com/wailsapp/wails/v2/pkg/options/assetserver"

	"github.com/miopunch/miopunch/internal/logutil"
)

//go:embed all:frontend/dist
var embeddedAssets embed.FS

func main() {
	initDesktopLogger()

	if err := runDesktop(); err != nil {
		logutil.Errorf("desktop startup failed: %v", err)
		reportStartupError(err)
		os.Exit(1)
	}
}

func runDesktop() (err error) {
	defer func() {
		if r := recover(); r != nil {
			logutil.Errorf("desktop runtime panic: %v\n%s", r, debug.Stack())
			err = fmt.Errorf("desktop runtime panic: %v", r)
		}
	}()

	if err := validatePlatformStartupEnvironment(); err != nil {
		return err
	}

	a := NewApp()

	appOptions := &options.App{
		Title:     "miopunch",
		Width:     1280,
		Height:    820,
		MinWidth:  960,
		MinHeight: 640,
		AssetServer: &assetserver.Options{
			Assets: embeddedAssets,
		},
		OnStartup:  a.startup,
		OnShutdown: a.shutdown,
		Bind:       []any{a},
		SingleInstanceLock: &options.SingleInstanceLock{
			UniqueId: "miopunch-desktop",
			OnSecondInstanceLaunch: func(options.SecondInstanceData) {
				a.restoreWindow()
			},
		},
	}
	configurePlatformOptions(appOptions)

	runErr := wails.Run(appOptions)
	if runErr != nil {
		return fmt.Errorf("desktop runtime failed: %w", runErr)
	}
	return nil
}
