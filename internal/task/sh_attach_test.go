package task

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func TestBridgeShellShellExitControlCompletesOK(t *testing.T) {
	ws := newShellBridgeTestWebSocket(t)
	localStream, remoteStream := net.Pipe()
	t.Cleanup(func() { _ = localStream.Close() })
	t.Cleanup(func() { _ = remoteStream.Close() })

	writeErrCh := make(chan error, 1)
	go func() {
		writeErrCh <- shellproto.WriteJSON(remoteStream, shellproto.Control{
			Op: shellproto.OpShellExit,
			OK: true,
		})
	}()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	got := bridgeShell(ctx, "task-test", "peer-test", ws, localStream, "local", "main")
	if got.reasonCode != poc.ReasonCodeOK {
		t.Fatalf("bridgeShell(shell_exit ok) ReasonCode = %q, want %q; facts=%v", got.reasonCode, poc.ReasonCodeOK, got.facts)
	}
	if got.exitCode != poc.ExitCodeOK {
		t.Fatalf("bridgeShell(shell_exit ok) ExitCode = %d, want %d", got.exitCode, poc.ExitCodeOK)
	}

	select {
	case err := <-writeErrCh:
		if err != nil {
			t.Fatalf("WriteJSON(shell_exit ok) error = %v, want nil", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatalf("WriteJSON(shell_exit ok) did not complete")
	}
}

func newShellBridgeTestWebSocket(t *testing.T) *websocket.Conn {
	t.Helper()

	serverConnCh := make(chan *websocket.Conn, 1)
	upgrader := websocket.Upgrader{
		CheckOrigin: func(r *http.Request) bool { return true },
	}
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := upgrader.Upgrade(w, r, nil)
		if err != nil {
			t.Errorf("Upgrade() error = %v, want nil", err)
			return
		}
		serverConnCh <- conn
	}))
	t.Cleanup(ts.Close)

	url := "ws" + strings.TrimPrefix(ts.URL, "http")
	clientConn, _, err := websocket.DefaultDialer.Dial(url, nil)
	if err != nil {
		t.Fatalf("Dial(%q) error = %v", url, err)
	}
	t.Cleanup(func() { _ = clientConn.Close() })

	select {
	case serverConn := <-serverConnCh:
		t.Cleanup(func() { _ = serverConn.Close() })
		return serverConn
	case <-time.After(2 * time.Second):
		t.Fatalf("server websocket was not accepted")
		return nil
	}
}
