package main

import (
	"strings"

	"github.com/miopunch/miopunch/internal/bundlepath"
	"github.com/miopunch/miopunch/internal/logutil"
)

const daemonLogFileName = "miopunch.log"

func initDaemonLogger(level string) {
	logPath, err := bundlepath.LogPath(daemonLogFileName)
	if err != nil {
		logutil.InitLogger("console", levelOrDefault(level), 0, true)
		logutil.Warnf("portable log unavailable: %v", err)
		return
	}

	logutil.InitLogger(logPath, levelOrDefault(level), 0, true)
	logutil.Infof("miopunch daemon log initialized: path=%s", logPath)
}

func levelOrDefault(level string) string {
	level = strings.ToLower(strings.TrimSpace(level))
	if level == "" {
		return "info"
	}
	return level
}

func logDaemonStatePath(statePath string) {
	statePath = strings.TrimSpace(statePath)
	if statePath == "" {
		return
	}
	logutil.Infof("miopunch daemon state path: path=%s", statePath)
}
