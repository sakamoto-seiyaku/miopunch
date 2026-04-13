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
	"os"
	"strings"
	"time"

	"github.com/256dpi/gomqtt/broker"
	"github.com/256dpi/gomqtt/transport"

	"github.com/miopunch/miopunch/event"
)

func mqttBrokerCmd(ctx context.Context, args []string) {
	fs := flag.NewFlagSet("mqtt-broker", flag.ExitOnError)
	listen := fs.String("listen", "0.0.0.0:1883", "listen address")
	_ = fs.Parse(args)

	em := event.NewEmitter(os.Stdout, "mqtt-broker")

	url := strings.TrimSpace(*listen)
	if url == "" {
		fmt.Fprintln(os.Stderr, "missing --listen")
		os.Exit(2)
	}
	if !strings.Contains(url, "://") {
		url = "tcp://" + url
	}

	server, err := transport.Launch(url)
	if err != nil {
		em.Fail(event.StageSupervisor, err, "mqtt broker listen failed", map[string]any{"url": url})
		os.Exit(1)
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
}
