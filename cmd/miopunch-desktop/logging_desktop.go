//go:build desktop

package main

import (
	"github.com/miopunch/miopunch/internal/bundlepath"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/sessionconfig"
)

const desktopLogFileName = "miopunch-desktop.log"

var desktopSessionConfigPath = bundlepath.SessionConfigPath

func initDesktopLogger() {
	level, configErr := desktopLogLevel()
	logPath, err := bundlepath.LogPath(desktopLogFileName)
	if err != nil {
		logutil.InitLogger("console", level, 0, true)
		logutil.Warnf("desktop portable log unavailable: %v", err)
		if configErr != nil {
			logutil.Warnf("desktop session log config unavailable: %v", configErr)
		}
		return
	}

	logutil.InitLogger(logPath, level, 0, true)
	if configErr != nil {
		logutil.Warnf("desktop session log config unavailable: %v", configErr)
	}
	logutil.Infof("miopunch desktop log initialized: path=%s log_level=%s", logPath, level)
}

func desktopLogLevel() (string, error) {
	path, err := desktopSessionConfigPath()
	if err != nil {
		return sessionconfig.DefaultLogLevel, err
	}
	config, err := sessionconfig.ReadFile(path)
	if err != nil {
		return sessionconfig.DefaultLogLevel, err
	}
	return config.Preferences.LogLevel, nil
}
