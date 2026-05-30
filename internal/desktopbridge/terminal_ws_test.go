package desktopbridge

import (
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

func TestTerminalWSBridgeRefusesMissingOrWrongToken(t *testing.T) {
	t.Parallel()

	b, err := NewTerminalWSBridge(func(context.Context, string) (ShellStream, error) {
		t.Fatalf("dial unexpectedly called")
		return nil, errors.New("unreachable")
	})
	if err != nil {
		t.Fatalf("NewTerminalWSBridge() error = %v", err)
	}
	t.Cleanup(func() { _ = b.Close() })

	d := websocket.Dialer{Subprotocols: []string{ShellSubprotocolV0}}

	{
		conn, resp, err := d.Dial(b.BaseURL()+"/api/v1/shell/s1/ws", nil)
		if conn != nil {
			_ = conn.Close()
		}
		if resp != nil {
			_ = resp.Body.Close()
		}
		if err == nil {
			t.Fatalf("Dial(missing token) error = nil, want non-nil")
		}
		if resp == nil || resp.StatusCode != http.StatusForbidden {
			got := 0
			if resp != nil {
				got = resp.StatusCode
			}
			t.Fatalf("Dial(missing token) status = %d, want %d", got, http.StatusForbidden)
		}
	}

	{
		conn, resp, err := d.Dial(b.BaseURL()+"/api/v1/shell/s1/ws?token=wrong", nil)
		if conn != nil {
			_ = conn.Close()
		}
		if resp != nil {
			_ = resp.Body.Close()
		}
		if err == nil {
			t.Fatalf("Dial(wrong token) error = nil, want non-nil")
		}
		if resp == nil || resp.StatusCode != http.StatusForbidden {
			got := 0
			if resp != nil {
				got = resp.StatusCode
			}
			t.Fatalf("Dial(wrong token) status = %d, want %d", got, http.StatusForbidden)
		}
	}
}

func TestTerminalWSBridgeProxiesBinaryAndTextToShellStream(t *testing.T) {
	t.Parallel()

	var (
		serverConn net.Conn
		ready      = make(chan struct{})
	)
	b, err := NewTerminalWSBridge(func(context.Context, string) (ShellStream, error) {
		localConn, remoteConn := net.Pipe()
		serverConn = remoteConn
		close(ready)
		return localConn, nil
	})
	if err != nil {
		t.Fatalf("NewTerminalWSBridge() error = %v", err)
	}
	t.Cleanup(func() {
		if serverConn != nil {
			_ = serverConn.Close()
		}
		_ = b.Close()
	})

	d := websocket.Dialer{Subprotocols: []string{ShellSubprotocolV0}}
	clientConn, resp, err := d.Dial(b.ShellURL("shell-1"), nil)
	if resp != nil {
		_ = resp.Body.Close()
	}
	if err != nil {
		t.Fatalf("Dial(bridge) error = %v", err)
	}
	t.Cleanup(func() { _ = clientConn.Close() })

	select {
	case <-ready:
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for backend stream")
	}

	{
		want := []byte("hello")
		if err := clientConn.WriteMessage(websocket.BinaryMessage, want); err != nil {
			t.Fatalf("WriteMessage(binary) error = %v", err)
		}

		got := make([]byte, len(want))
		if _, err := io.ReadFull(serverConn, got); err != nil {
			t.Fatalf("ReadFull(binary) error = %v", err)
		}
		if string(got) != string(want) {
			t.Fatalf("backend payload = %q, want %q", string(got), string(want))
		}
	}

	{
		want := `{"op":"winsize","cols":80,"rows":24}`
		if err := clientConn.WriteMessage(websocket.TextMessage, []byte(want)); err != nil {
			t.Fatalf("WriteMessage(text) error = %v", err)
		}

		got := make([]byte, len(want))
		if _, err := io.ReadFull(serverConn, got); err != nil {
			t.Fatalf("ReadFull(text) error = %v", err)
		}
		if string(got) != want {
			t.Fatalf("backend payload = %q, want %q", string(got), want)
		}
	}

	{
		want := []byte("shell output")
		if _, err := serverConn.Write(want); err != nil {
			t.Fatalf("backend Write() error = %v", err)
		}

		_ = clientConn.SetReadDeadline(time.Now().Add(2 * time.Second))
		mt, got, err := clientConn.ReadMessage()
		if err != nil {
			t.Fatalf("ReadMessage() error = %v", err)
		}
		if mt != websocket.BinaryMessage {
			t.Fatalf("ReadMessage() messageType = %d, want %d", mt, websocket.BinaryMessage)
		}
		if string(got) != string(want) {
			t.Fatalf("ReadMessage() payload = %q, want %q", string(got), string(want))
		}
	}
}
