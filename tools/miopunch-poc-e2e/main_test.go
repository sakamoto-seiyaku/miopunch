package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/shellproto"
	"github.com/miopunch/miopunch/internal/task"
)

func TestShAttach_SendsAndObservesMarker(t *testing.T) {
	t.Parallel()

	socketPath := startLocalAPIServer(t, serveEchoShell)

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
	if got.TaskID == "" {
		t.Fatalf("result.TaskID is empty, result=%+v", got)
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

	socketPath := startLocalAPIServer(t, serveRejectSHInUseShell)

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

func startLocalAPIServer(t *testing.T, serveShell func(conn io.ReadWriteCloser)) string {
	t.Helper()

	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Peers: map[string]pocstate.PeerConfig{
			"peer1": {},
		},
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}

	mgr := task.NewManagerWithStatePath(statePath)
	t.Cleanup(mgr.Close)
	mgr.SetDialPeerStreamHook(func(ctx context.Context, taskID string, peerID string, cfg pocstate.PeerConfig) (io.ReadWriteCloser, error) {
		clientConn, serverConn := net.Pipe()
		go serveShell(serverConn)
		return clientConn, nil
	})

	socketPath := filepath.Join(t.TempDir(), "localapi.sock")
	ln, err := net.Listen("unix", socketPath)
	if err != nil {
		t.Fatalf("net.Listen(unix, %q) error = %v", socketPath, err)
	}
	t.Cleanup(func() { _ = ln.Close() })

	api := localapi.NewServer(localapi.ListenModeUser, mgr)
	srv := &http.Server{Handler: api.Handler()}
	go func() { _ = srv.Serve(ln) }()
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		defer cancel()
		_ = srv.Shutdown(ctx)
	})

	return socketPath
}

func serveEchoShell(conn io.ReadWriteCloser) {
	defer conn.Close()

	var hello shellproto.Control
	if err := shellproto.ReadJSON(conn, &hello); err != nil {
		return
	}
	_ = shellproto.WriteJSON(conn, shellproto.Control{Op: shellproto.OpHello, OK: true})

	var req shellproto.Control
	if err := shellproto.ReadJSON(conn, &req); err != nil {
		return
	}
	_ = shellproto.WriteJSON(conn, shellproto.Control{
		Op:      shellproto.OpShAttach,
		OK:      true,
		Target:  strings.TrimSpace(req.Target),
		Session: strings.TrimSpace(req.Session),
	})

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

func serveRejectSHInUseShell(conn io.ReadWriteCloser) {
	defer conn.Close()

	var hello shellproto.Control
	if err := shellproto.ReadJSON(conn, &hello); err != nil {
		return
	}
	_ = shellproto.WriteJSON(conn, shellproto.Control{Op: shellproto.OpHello, OK: true})

	var req shellproto.Control
	if err := shellproto.ReadJSON(conn, &req); err != nil {
		return
	}
	_ = shellproto.WriteJSON(conn, shellproto.Control{
		Op: shellproto.OpShAttach,
		OK: false,
		Error: &shellproto.ControlError{
			ReasonCode: "SH_IN_USE",
			Message:    "shell already in use",
		},
	})
}
