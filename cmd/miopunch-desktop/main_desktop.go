//go:build desktop

package main

import (
	"embed"
	"os"

	"github.com/wailsapp/wails/v2"
	"github.com/wailsapp/wails/v2/pkg/options"
	"github.com/wailsapp/wails/v2/pkg/options/assetserver"
)

//go:embed all:frontend/dist
var embeddedAssets embed.FS

func main() {
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

	err := wails.Run(appOptions)
	if err != nil {
		reportStartupError(err)
		os.Exit(1)
	}
}
