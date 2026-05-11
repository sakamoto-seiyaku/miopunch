//go:build desktop

package main

import (
	"github.com/miopunch/miopunch/internal/bundlepath"
	"github.com/miopunch/miopunch/internal/logutil"
)

const desktopLogFileName = "miopunch-desktop.log"

func initDesktopLogger() {
	logPath, err := bundlepath.LogPath(desktopLogFileName)
	if err != nil {
		logutil.InitLogger("console", "info", 0, true)
		logutil.Warnf("desktop portable log unavailable: %v", err)
		return
	}

	logutil.InitLogger(logPath, "info", 0, true)
	logutil.Infof("miopunch desktop log initialized: path=%s", logPath)
}
