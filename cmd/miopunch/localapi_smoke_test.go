package main

import (
	"bytes"
	"context"
	"encoding/json"
	"net"
	"net/http"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/256dpi/gomqtt/broker"
	"github.com/256dpi/gomqtt/transport"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/task"
)

func TestCLI_Smoke_LocalAPI_Invite_JSONEnvelope(t *testing.T) {
	t.Parallel()

	socketPath := filepath.Join(t.TempDir(), "localapi.sock")
	ln, err := net.Listen("unix", socketPath)
	if err != nil {
		t.Fatalf("listen unix: %v", err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	brokerAddr := startTCPMQTTBroker(t)
	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			MQTTBroker:  brokerAddr,
			TopicPrefix: "miopunch/test",
			DataProto:   "quic",
			QUICCC:      "bbr",
		},
		Peers: map[string]pocstate.PeerConfig{},
	}); err != nil {
		t.Fatalf("seed state.json: %v", err)
	}
	mgr := task.NewManagerWithStatePath(statePath)
	t.Cleanup(mgr.Close)

	api := localapi.NewServer(localapi.ListenModeUser, mgr)
	srv := &http.Server{Handler: api.Handler()}
	go func() { _ = srv.Serve(ln) }()
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		defer cancel()
		_ = srv.Shutdown(ctx)
	})

	client, err := localapi.NewClient(localapi.Addr{Transport: localapi.TransportUnix, Path: socketPath})
	if err != nil {
		t.Fatalf("new localapi client: %v", err)
	}
	deadline := time.Now().Add(1 * time.Second)
	for {
		if err := client.ProbeStatus(context.Background()); err == nil {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("localapi status not reachable within deadline")
		}
		time.Sleep(10 * time.Millisecond)
	}

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	exitCode := run([]string{"--format", "json", "--localapi", "unix:" + socketPath, "invite"}, &stdout, &stderr)
	if exitCode != 0 {
		t.Fatalf("run(invite --format json) exitCode=%d want %d, stderr=%s", exitCode, 0, stderr.String())
	}

	out := stdout.String()
	if strings.Count(out, "\n") != 1 {
		t.Fatalf("expected single-line JSON output, got:\n%s", out)
	}

	var env poc.EnvelopeJSONV0
	if err := json.Unmarshal([]byte(out), &env); err != nil {
		t.Fatalf("decode JSON envelope: %v\nraw=%s", err, out)
	}

	if env.Format != poc.JSONFormatV0 {
		t.Fatalf("format=%q want %q", env.Format, poc.JSONFormatV0)
	}
	if env.TaskID == "" {
		t.Fatalf("task_id is empty, raw=%s", out)
	}
	if env.Kind != "invite" {
		t.Fatalf("kind=%q want %q", env.Kind, "invite")
	}
	if env.Status != "done" {
		t.Fatalf("status=%q want %q", env.Status, "done")
	}
	if env.Stage == "" {
		t.Fatalf("stage is empty, raw=%s", out)
	}
	if env.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("reason_code=%q want %q", env.ReasonCode, poc.ReasonCodeOK)
	}
	if env.ExitCode != poc.ExitCodeOK {
		t.Fatalf("exit_code=%d want %d", env.ExitCode, poc.ExitCodeOK)
	}
	if len(env.Facts) == 0 {
		t.Fatalf("facts is empty, raw=%s", out)
	}
	if env.Suggestions == nil {
		t.Fatalf("suggestions is nil, raw=%s", out)
	}
}

func startTCPMQTTBroker(t *testing.T) string {
	t.Helper()

	server, err := transport.Launch("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("transport.Launch(tcp://127.0.0.1:0) error = %v", err)
	}
	backend := broker.NewMemoryBackend()
	engine := broker.NewEngine(backend)
	engine.Accept(server)

	t.Cleanup(func() {
		_ = server.Close()
		backend.Close(500 * time.Millisecond)
		engine.Close()
	})

	return server.Addr().String()
}
