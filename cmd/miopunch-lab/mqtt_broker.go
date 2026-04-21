// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

package main

import (
	"context"
	"flag"
	"fmt"
	"io"
	"strings"
	"time"

	"github.com/256dpi/gomqtt/broker"
	"github.com/256dpi/gomqtt/transport"

	"github.com/miopunch/miopunch/event"
)

func mqttBrokerCmd(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	fs := flag.NewFlagSet("mqtt-broker", flag.ContinueOnError)
	fs.SetOutput(stderr)
	listen := fs.String("listen", "0.0.0.0:1883", "listen address")
	logLevel := addLogLevelFlag(fs)
	if err := fs.Parse(args); err != nil {
		if err == flag.ErrHelp {
			return 0
		}
		return 2
	}
	applyLogLevel(*logLevel)

	em := event.NewEmitter(stdout, "mqtt-broker")

	url := strings.TrimSpace(*listen)
	if url == "" {
		fmt.Fprintln(stderr, "missing --listen")
		return 2
	}
	if !strings.Contains(url, "://") {
		url = "tcp://" + url
	}

	server, err := transport.Launch(url)
	if err != nil {
		em.Fail(event.StageSupervisor, err, "mqtt broker listen failed", map[string]any{"url": url})
		return 1
	}

	backend := broker.NewMemoryBackend()
	engine := broker.NewEngine(backend)
	engine.OnError = func(err error) {
		em.Fail(event.StageSupervisor, err, "mqtt broker error", nil)
	}
	engine.Accept(server)

	em.OK(event.StageSignaling, "mqtt broker listening", map[string]any{"url": url})

	<-ctx.Done()

	backend.Close(5 * time.Second)
	_ = server.Close()
	engine.Close()
	return 0
}
