package main

import (
	"strings"

	"github.com/miopunch/miopunch/internal/bundlepath"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/sessionconfig"
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
	level, err := sessionconfig.NormalizeLogLevel(level)
	if err != nil {
		return sessionconfig.DefaultLogLevel
	}
	return level
}

func applySessionLogLevel(opt upOptions) (upOptions, error) {
	if !opt.Session || strings.TrimSpace(opt.LogLevel) != "" {
		return opt, nil
	}
	path, err := bundlepath.SessionConfigPath()
	if err != nil {
		return opt, err
	}
	config, err := sessionconfig.ReadFile(path)
	if err != nil {
		return opt, err
	}
	opt.LogLevel = config.Preferences.LogLevel
	return opt, nil
}

func logDaemonStatePath(statePath string) {
	statePath = strings.TrimSpace(statePath)
	if statePath == "" {
		return
	}
	logutil.Infof("miopunch daemon state path: path=%s", statePath)
}
