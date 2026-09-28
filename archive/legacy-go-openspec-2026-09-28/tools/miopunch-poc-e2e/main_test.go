package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func TestShAttach_SendsAndObservesMarker(t *testing.T) {
	t.Parallel()

	socketPath := startFakeLocalAPIServer(t, fakeLocalAPIConfig{
		actionResult: pocruntime.ActionResult{
			Stage:          pocruntime.StageShell,
			ReasonCode:     poc.ReasonCodeOK,
			ExitCode:       poc.ExitCodeOK,
			ShellSessionID: "shell-1",
		},
		shellHandler: serveEchoShell,
	})

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	exitCode := run([]string{
		"sh-attach",
		"--localapi", "unix:" + socketPath,
		"--peer-id", "peer1",
		"--target", "local",
		"--session", "main",
		"--send", "hello-marker",
		"--expect", "hello-marker",
		"--timeout", "5s",
	}, &stdout, &stderr)
	if exitCode != 0 {
		t.Fatalf("run(sh-attach) exitCode=%d, want 0, stderr=%s stdout=%s", exitCode, stderr.String(), stdout.String())
	}

	var got shAttachResult
	if err := json.Unmarshal(stdout.Bytes(), &got); err != nil {
		t.Fatalf("json.Unmarshal(stdout) error = %v, stdout=%s", err, stdout.String())
	}
	if !got.OK {
		t.Fatalf("result.OK = false, want true, result=%+v stderr=%s", got, stderr.String())
	}
	if got.ShellSessionID != "shell-1" {
		t.Fatalf("result.ShellSessionID = %q, want %q", got.ShellSessionID, "shell-1")
	}
	if got.PeerID != "peer1" {
		t.Fatalf("result.PeerID = %q, want %q", got.PeerID, "peer1")
	}
	if got.ObservedBytes == 0 {
		t.Fatalf("result.ObservedBytes = %d, want > 0", got.ObservedBytes)
	}
	if got.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("result.ReasonCode = %q, want %q", got.ReasonCode, poc.ReasonCodeOK)
	}
}

func TestShAttach_ValidatesRequiredArgs(t *testing.T) {
	t.Parallel()

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	exitCode := run([]string{
		"sh-attach",
		"--send", "hello",
		"--expect", "hello",
	}, &stdout, &stderr)
	if exitCode == 0 {
		t.Fatalf("run(sh-attach missing peer) exitCode=0, want non-zero")
	}

	var got shAttachResult
	if err := json.Unmarshal(stdout.Bytes(), &got); err != nil {
		t.Fatalf("json.Unmarshal(stdout) error = %v, stdout=%s", err, stdout.String())
	}
	if got.OK {
		t.Fatalf("result.OK = true, want false, result=%+v", got)
	}
	if got.ReasonCode != poc.ReasonCodeBadRequest {
		t.Fatalf("result.ReasonCode = %q, want %q", got.ReasonCode, poc.ReasonCodeBadRequest)
	}
	if !strings.Contains(stderr.String(), "missing --peer-id") {
		t.Fatalf("stderr = %q, want contains %q", stderr.String(), "missing --peer-id")
	}
}

func TestShAttach_PropagatesTaskFailureReason(t *testing.T) {
	t.Parallel()

	socketPath := startFakeLocalAPIServer(t, fakeLocalAPIConfig{
		actionError: &localapi.ErrorResponse{
			Stage:      string(pocruntime.StageShell),
			ReasonCode: poc.ReasonCodeSHInUse,
			ExitCode:   poc.ExitCodeConflict,
			Message:    "shell already in use",
		},
	})

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	exitCode := run([]string{
		"sh-attach",
		"--localapi", "unix:" + socketPath,
		"--peer-id", "peer1",
		"--target", "local",
		"--session", "main",
		"--send", "marker",
		"--expect", "marker",
		"--timeout", "2s",
	}, &stdout, &stderr)
	if exitCode != int(poc.ExitCodeConflict) {
		t.Fatalf("run(sh-attach conflict) exitCode=%d, want %d, stderr=%s stdout=%s", exitCode, poc.ExitCodeConflict, stderr.String(), stdout.String())
	}

	var got shAttachResult
	if err := json.Unmarshal(stdout.Bytes(), &got); err != nil {
		t.Fatalf("json.Unmarshal(stdout) error = %v, stdout=%s", err, stdout.String())
	}
	if got.OK {
		t.Fatalf("result.OK = true, want false, result=%+v", got)
	}
	if got.ReasonCode != poc.ReasonCodeSHInUse {
		t.Fatalf("result.ReasonCode = %q, want %q, result=%+v stderr=%s", got.ReasonCode, poc.ReasonCodeSHInUse, got, stderr.String())
	}
}

func TestStatus_ReportsHealthyLocalAPI(t *testing.T) {
	t.Parallel()

	socketPath := startFakeLocalAPIServer(t, fakeLocalAPIConfig{})

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	exitCode := run([]string{
		"status",
		"--localapi", "unix:" + socketPath,
	}, &stdout, &stderr)
	if exitCode != 0 {
		t.Fatalf("run(status) exitCode=%d, want 0, stderr=%s stdout=%s", exitCode, stderr.String(), stdout.String())
	}

	var got statusResult
	if err := json.Unmarshal(stdout.Bytes(), &got); err != nil {
		t.Fatalf("json.Unmarshal(stdout) error = %v, stdout=%s", err, stdout.String())
	}
	if !got.OK {
		t.Fatalf("result.OK = false, want true, result=%+v stderr=%s", got, stderr.String())
	}
	if got.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("result.ReasonCode = %q, want %q", got.ReasonCode, poc.ReasonCodeOK)
	}
	if got.Mode != "user" {
		t.Fatalf("result.Mode = %q, want %q", got.Mode, "user")
	}
}

type fakeLocalAPIConfig struct {
	actionResult pocruntime.ActionResult
	actionError  *localapi.ErrorResponse
	shellHandler func(io.ReadWriteCloser)
}

func startFakeLocalAPIServer(t *testing.T, cfg fakeLocalAPIConfig) string {
	t.Helper()

	socketPath := filepath.Join(t.TempDir(), "localapi.sock")
	ln, err := net.Listen("unix", socketPath)
	if err != nil {
		t.Fatalf("net.Listen(unix, %q) error = %v", socketPath, err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	go serveFakeLocalAPI(ctx, ln, cfg)

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

	return socketPath
}

func serveFakeLocalAPI(ctx context.Context, ln net.Listener, cfg fakeLocalAPIConfig) {
	for {
		conn, err := ln.Accept()
		if err != nil {
			select {
			case <-ctx.Done():
				return
			default:
				return
			}
		}
		go handleFakeLocalAPIConn(conn, cfg)
	}
}

func handleFakeLocalAPIConn(conn net.Conn, cfg fakeLocalAPIConfig) {
	defer conn.Close()

	reader := bufio.NewReader(conn)
	var preface struct {
		Version        int    `json:"version"`
		Channel        string `json:"channel"`
		ShellSessionID string `json:"shell_session_id,omitempty"`
	}
	prefaceLine, err := reader.ReadBytes('\n')
	if err != nil {
		return
	}
	if err := json.Unmarshal(bytes.TrimSpace(prefaceLine), &preface); err != nil {
		return
	}
	if preface.Version != 1 {
		return
	}

	switch strings.TrimSpace(preface.Channel) {
	case "rpc":
		handleFakeRPC(conn, reader, cfg)
	case "shell":
		if cfg.shellHandler != nil {
			cfg.shellHandler(&bufferedConn{Conn: conn, reader: reader})
		}
	}
}

func handleFakeRPC(conn net.Conn, reader *bufio.Reader, cfg fakeLocalAPIConfig) {
	var request struct {
		JSONRPC string          `json:"jsonrpc"`
		ID      json.RawMessage `json:"id"`
		Method  string          `json:"method"`
		Params  json.RawMessage `json:"params,omitempty"`
	}
	requestLine, err := reader.ReadBytes('\n')
	if err != nil {
		return
	}
	if err := json.Unmarshal(bytes.TrimSpace(requestLine), &request); err != nil {
		return
	}

	response := map[string]any{
		"jsonrpc": "2.0",
		"id":      request.ID,
	}
	switch strings.TrimSpace(request.Method) {
	case "status":
		response["result"] = map[string]any{
			"version":    "test",
			"started_at": time.Now().UTC(),
			"uptime_ms":  1,
			"mode":       "user",
		}
	case "action":
		if cfg.actionError != nil {
			response["error"] = map[string]any{
				"code":    -int(cfg.actionError.ExitCode),
				"message": cfg.actionError.Message,
				"data":    cfg.actionError,
			}
		} else {
			response["result"] = cfg.actionResult
		}
	default:
		response["error"] = map[string]any{
			"code":    -32601,
			"message": "method not found",
		}
	}
	_ = json.NewEncoder(conn).Encode(response)
}

func serveEchoShell(conn io.ReadWriteCloser) {
	defer conn.Close()

	for {
		kind, payload, err := shellproto.ReadFrame(conn)
		if err != nil {
			return
		}
		if kind != shellproto.KindData {
			continue
		}
		if err := shellproto.WriteFrame(conn, shellproto.KindData, payload); err != nil {
			return
		}
	}
}

type bufferedConn struct {
	net.Conn
	reader *bufio.Reader
}

func (c *bufferedConn) Read(p []byte) (int, error) {
	if c == nil || c.reader == nil {
		return 0, io.EOF
	}
	return c.reader.Read(p)
}
