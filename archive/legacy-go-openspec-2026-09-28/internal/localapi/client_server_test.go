//go:build !windows

package localapi

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func TestClientServer_StatusSnapshotEventsAndActionFailure(t *testing.T) {
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

	server := NewServer(ListenModeUser, runtimeInstance)
	t.Cleanup(func() { _ = server.Close() })
	go func() { _ = server.Serve(listener) }()

	client, err := NewClient(Addr{Transport: TransportUnix, Path: socketPath})
	if err != nil {
		t.Fatalf("NewClient() error = %v, want nil", err)
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

	status, err := client.GetStatus(context.Background())
	if err != nil {
		t.Fatalf("GetStatus() error = %v, want nil", err)
	}
	if status.Mode != ListenModeUser {
		t.Fatalf("GetStatus().Mode = %q, want %q", status.Mode, ListenModeUser)
	}

	snapshot, err := client.GetSnapshot(context.Background())
	if err != nil {
		t.Fatalf("GetSnapshot() error = %v, want nil", err)
	}
	if snapshot.Stage != pocruntime.StageNetwork {
		t.Fatalf("GetSnapshot().Stage = %q, want %q", snapshot.Stage, pocruntime.StageNetwork)
	}
	if snapshot.Config.Effective.Preferences.LogLevel != "info" {
		t.Fatalf("GetSnapshot().Config.Effective.Preferences.LogLevel = %q, want info", snapshot.Config.Effective.Preferences.LogLevel)
	}

	snapshot, err = client.SetLogLevel(context.Background(), "debug")
	if err != nil {
		t.Fatalf("SetLogLevel(debug) error = %v, want nil", err)
	}
	if snapshot.Config.Effective.Preferences.LogLevel != "debug" {
		t.Fatalf("SetLogLevel(debug).Config.Effective.Preferences.LogLevel = %q, want debug", snapshot.Config.Effective.Preferences.LogLevel)
	}

	var apiErr *APIError
	_, err = client.SetLogLevel(context.Background(), "verbose")
	if !errors.As(err, &apiErr) {
		t.Fatalf("SetLogLevel(verbose) error type = %T, want *APIError", err)
	}
	if apiErr.Response.ReasonCode != poc.ReasonCodeBadRequest {
		t.Fatalf("SetLogLevel(verbose) reason_code = %q, want %q", apiErr.Response.ReasonCode, poc.ReasonCodeBadRequest)
	}

	events, err := client.OpenEvents(context.Background())
	if err != nil {
		t.Fatalf("OpenEvents() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = events.Close() })

	line, err := bufio.NewReader(events).ReadBytes('\n')
	if err != nil {
		t.Fatalf("ReadBytes(event) error = %v, want nil", err)
	}
	var event Event
	if err := json.Unmarshal(line, &event); err != nil {
		t.Fatalf("json.Unmarshal(event) error = %v, want nil", err)
	}
	if event.Kind != "snapshot" {
		t.Fatalf("event.Kind = %q, want %q", event.Kind, "snapshot")
	}

	_, err = client.Action(context.Background(), "join", pocruntime.JoinArgs{})
	if !errors.As(err, &apiErr) {
		t.Fatalf("Action(join) error type = %T, want *APIError", err)
	}
	if apiErr.Response.ReasonCode != poc.ReasonCodeBadRequest {
		t.Fatalf("Action(join) reason_code = %q, want %q", apiErr.Response.ReasonCode, poc.ReasonCodeBadRequest)
	}
	if apiErr.Response.Stage != string(pocruntime.StageEnroll) {
		t.Fatalf("Action(join) stage = %q, want %q", apiErr.Response.Stage, pocruntime.StageEnroll)
	}
}

func TestClientActionDeadlineExceeded(t *testing.T) {
	t.Parallel()

	socketPath := filepath.Join(t.TempDir(), "localapi.sock")
	listener, err := net.Listen("unix", socketPath)
	if err != nil {
		t.Fatalf("net.Listen(unix) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = listener.Close() })

	done := make(chan struct{})
	go func() {
		defer close(done)
		conn, err := listener.Accept()
		if err != nil {
			return
		}
		defer conn.Close()

		reader := bufio.NewReader(conn)
		if _, err := reader.ReadBytes('\n'); err != nil {
			return
		}
		_, _ = io.ReadAll(reader)
	}()
	t.Cleanup(func() { <-done })

	client, err := NewClient(Addr{Transport: TransportUnix, Path: socketPath})
	if err != nil {
		t.Fatalf("NewClient() error = %v, want nil", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()

	_, err = client.Action(ctx, "ls", nil)
	if !errors.Is(err, context.DeadlineExceeded) && !strings.Contains(err.Error(), context.DeadlineExceeded.Error()) {
		t.Fatalf("Action(deadline) error = %v, want context deadline exceeded", err)
	}
}

func TestServerShellChannelPreservesBufferedFirstFrame(t *testing.T) {
	t.Parallel()

	serverConn, clientConn := net.Pipe()
	defer serverConn.Close()
	defer clientConn.Close()

	var (
		gotPreface channelPreface
		gotPayload []byte
		wg         sync.WaitGroup
	)

	wg.Add(1)
	go func() {
		defer wg.Done()
		reader := bufio.NewReader(serverConn)
		line, err := reader.ReadBytes('\n')
		if err != nil {
			return
		}
		if err := json.Unmarshal(trimJSONLine(line), &gotPreface); err != nil {
			return
		}
		stream := &bufferedConn{Conn: serverConn, reader: reader}
		_, payload, err := shellproto.ReadFrame(stream)
		if err != nil {
			return
		}
		gotPayload = append([]byte(nil), payload...)
	}()

	if err := json.NewEncoder(clientConn).Encode(channelPreface{
		Version:        protocolVersion,
		Channel:        channelShell,
		ShellSessionID: "shell-1",
	}); err != nil {
		t.Fatalf("Encode(preface) error = %v, want nil", err)
	}
	if err := shellproto.WriteFrame(clientConn, shellproto.KindData, []byte("hello-shell")); err != nil {
		t.Fatalf("WriteFrame() error = %v, want nil", err)
	}
	_ = clientConn.Close()
	wg.Wait()

	if gotPreface.Channel != channelShell {
		t.Fatalf("preface.Channel = %q, want %q", gotPreface.Channel, channelShell)
	}
	if string(gotPayload) != "hello-shell" {
		t.Fatalf("payload = %q, want %q", string(gotPayload), "hello-shell")
	}
}
