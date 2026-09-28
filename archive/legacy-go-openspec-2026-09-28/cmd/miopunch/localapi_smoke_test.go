//go:build !windows

package main

import (
	"bytes"
	"context"
	"encoding/json"
	"net"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
)

func TestCLI_Smoke_LocalAPI_InitNetwork_JSONEnvelope(t *testing.T) {
	t.Parallel()

	runtimeInstance, err := pocruntime.Open(pocruntime.Options{Root: t.TempDir()})
	if err != nil {
		t.Fatalf("runtime.Open() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = runtimeInstance.Close() })

	socketPath := filepath.Join(t.TempDir(), "localapi.sock")
	listener, err := net.Listen("unix", socketPath)
	if err != nil {
		t.Fatalf("net.Listen(unix) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = listener.Close() })

	server := localapi.NewServer(localapi.ListenModeUser, runtimeInstance)
	t.Cleanup(func() { _ = server.Close() })
	go func() { _ = server.Serve(listener) }()

	client, err := localapi.NewClient(localapi.Addr{Transport: localapi.TransportUnix, Path: socketPath})
	if err != nil {
		t.Fatalf("localapi.NewClient() error = %v, want nil", err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for {
		if err := client.ProbeStatus(context.Background()); err == nil {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("ProbeStatus() did not succeed before deadline")
		}
		time.Sleep(20 * time.Millisecond)
	}

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	exitCode := run([]string{"--format", "json", "--localapi", "unix:" + socketPath, "init-network"}, &stdout, &stderr)
	if exitCode != 0 {
		t.Fatalf("run(init-network --format json) exitCode = %d, want 0, stderr=%s", exitCode, stderr.String())
	}

	out := stdout.String()
	if strings.Count(out, "\n") != 1 {
		t.Fatalf("expected single-line JSON output, got:\n%s", out)
	}

	var env poc.EnvelopeJSONV0
	if err := json.Unmarshal([]byte(out), &env); err != nil {
		t.Fatalf("json.Unmarshal(envelope) error = %v, raw=%s", err, out)
	}
	if env.Format != poc.JSONFormatV0 {
		t.Fatalf("env.Format = %q, want %q", env.Format, poc.JSONFormatV0)
	}
	if env.Kind != "init-network" {
		t.Fatalf("env.Kind = %q, want %q", env.Kind, "init-network")
	}
	if env.Status != "done" {
		t.Fatalf("env.Status = %q, want %q", env.Status, "done")
	}
	if env.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("env.ReasonCode = %q, want %q", env.ReasonCode, poc.ReasonCodeOK)
	}
	if env.ExitCode != poc.ExitCodeOK {
		t.Fatalf("env.ExitCode = %d, want %d", env.ExitCode, poc.ExitCodeOK)
	}
	if env.Stage == "" {
		t.Fatalf("env.Stage = empty, want non-empty")
	}
	if len(env.Facts) == 0 {
		t.Fatalf("env.Facts = empty, want non-empty")
	}
	if env.Suggestions == nil {
		t.Fatalf("env.Suggestions = nil, want non-nil")
	}
}
