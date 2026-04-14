package main

import (
	"flag"

	"github.com/miopunch/miopunch/internal/logutil"
)

func addLogLevelFlag(fs *flag.FlagSet) *string {
	return fs.String("log-level", "info", "internal log level: trace|debug|info|warn|error")
}

func applyLogLevel(level string) {
	logutil.SetLevel(level)
}
