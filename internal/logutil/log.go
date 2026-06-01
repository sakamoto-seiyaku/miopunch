// Copyright 2016 fatedier, fatedier@gmail.com
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

package logutil

import (
	"bytes"
	"os"
	"sync"

	"github.com/fatedier/golib/log"
)

var (
	TraceLevel = log.TraceLevel
	DebugLevel = log.DebugLevel
	InfoLevel  = log.InfoLevel
	WarnLevel  = log.WarnLevel
	ErrorLevel = log.ErrorLevel
)

var Logger *log.Logger
var loggerMu sync.RWMutex

func init() {
	Logger = log.New(
		log.WithCaller(true),
		log.AddCallerSkip(1),
		log.WithLevel(log.InfoLevel),
	)
}

func InitLogger(logPath string, levelStr string, maxDays int, disableLogColor bool) {
	options := []log.Option{}
	if logPath == "console" {
		if !disableLogColor {
			options = append(options,
				log.WithOutput(log.NewConsoleWriter(log.ConsoleConfig{
					Colorful: true,
				}, os.Stdout)),
			)
		}
	} else {
		writer := log.NewRotateFileWriter(log.RotateFileConfig{
			FileName: logPath,
			Mode:     log.RotateFileModeDaily,
			MaxDays:  maxDays,
		})
		writer.Init()
		options = append(options, log.WithOutput(writer))
	}

	level, err := log.ParseLevel(levelStr)
	if err != nil {
		level = log.InfoLevel
	}
	options = append(options, log.WithLevel(level))
	loggerMu.Lock()
	defer loggerMu.Unlock()
	Logger = Logger.WithOptions(options...)
}

func SetLevel(levelStr string) {
	level, err := log.ParseLevel(levelStr)
	if err != nil {
		level = log.InfoLevel
	}
	loggerMu.Lock()
	defer loggerMu.Unlock()
	Logger = Logger.WithOptions(log.WithLevel(level))
}

func Errorf(format string, v ...any) {
	loggerMu.RLock()
	defer loggerMu.RUnlock()
	Logger.Errorf(format, v...)
}

func Warnf(format string, v ...any) {
	loggerMu.RLock()
	defer loggerMu.RUnlock()
	Logger.Warnf(format, v...)
}

func Infof(format string, v ...any) {
	loggerMu.RLock()
	defer loggerMu.RUnlock()
	Logger.Infof(format, v...)
}

func Debugf(format string, v ...any) {
	loggerMu.RLock()
	defer loggerMu.RUnlock()
	Logger.Debugf(format, v...)
}

func Tracef(format string, v ...any) {
	loggerMu.RLock()
	defer loggerMu.RUnlock()
	Logger.Tracef(format, v...)
}

func Logf(level log.Level, offset int, format string, v ...any) {
	loggerMu.RLock()
	defer loggerMu.RUnlock()
	Logger.Logf(level, offset, format, v...)
}

type WriteLogger struct {
	level  log.Level
	offset int
}

func NewWriteLogger(level log.Level, offset int) *WriteLogger {
	return &WriteLogger{
		level:  level,
		offset: offset,
	}
}

func (w *WriteLogger) Write(p []byte) (n int, err error) {
	loggerMu.RLock()
	defer loggerMu.RUnlock()
	Logger.Log(w.level, w.offset, string(bytes.TrimRight(p, "\n")))
	return len(p), nil
}
