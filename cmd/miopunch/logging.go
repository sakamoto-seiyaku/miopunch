package main

import (
	"strings"

	"github.com/miopunch/miopunch/internal/bundlepath"
	"github.com/miopunch/miopunch/internal/logutil"
)

const daemonLogFileName = "miopunch.log"

func initDaemonLogger() {
	logPath, err := bundlepath.LogPath(daemonLogFileName)
	if err != nil {
		logutil.InitLogger("console", "info", 0, true)
		logutil.Warnf("portable log unavailable: %v", err)
		return
	}

	logutil.InitLogger(logPath, "info", 0, true)
	logutil.Infof("miopunch daemon log initialized: path=%s", logPath)
}

func logDaemonStatePath(statePath string) {
	statePath = strings.TrimSpace(statePath)
	if statePath == "" {
		return
	}
	logutil.Infof("miopunch daemon state path: path=%s", statePath)
}
