package desktopbridge

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

func TestTerminalWSBridge_RefusesMissingOrWrongToken(t *testing.T) {
	t.Parallel()

	b, err := NewTerminalWSBridge(func(context.Context, string) (*websocket.Conn, error) {
		t.Fatalf("dial unexpectedly called")
		return nil, errors.New("unreachable")
	})
	if err != nil {
		t.Fatalf("NewTerminalWSBridge() error = %v", err)
	}
	t.Cleanup(func() { _ = b.Close() })

	d := websocket.Dialer{Subprotocols: []string{ShellSubprotocolV0}}

	{
		conn, resp, err := d.Dial(b.BaseURL()+"/api/v0/tasks/t/ws", nil)
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
		conn, resp, err := d.Dial(b.BaseURL()+"/api/v0/tasks/t/ws?token=wrong", nil)
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

func TestTerminalWSBridge_ProxiesBinaryAndText(t *testing.T) {
	t.Parallel()

	upgrader := websocket.Upgrader{
		Subprotocols: []string{ShellSubprotocolV0},
		CheckOrigin:  func(*http.Request) bool { return true },
	}

	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := upgrader.Upgrade(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.Close() }()

		for {
			mt, payload, err := conn.ReadMessage()
			if err != nil {
				return
			}
			if err := conn.WriteMessage(mt, payload); err != nil {
				return
			}
		}
	}))
	t.Cleanup(backend.Close)

	backendURL := "ws" + strings.TrimPrefix(backend.URL, "http")

	b, err := NewTerminalWSBridge(func(ctx context.Context, taskID string) (*websocket.Conn, error) {
		_ = taskID

		d := websocket.Dialer{Subprotocols: []string{ShellSubprotocolV0}}
		conn, resp, err := d.DialContext(ctx, backendURL, nil)
		if resp != nil {
			_ = resp.Body.Close()
		}
		return conn, err
	})
	if err != nil {
		t.Fatalf("NewTerminalWSBridge() error = %v", err)
	}
	t.Cleanup(func() { _ = b.Close() })

	d := websocket.Dialer{Subprotocols: []string{ShellSubprotocolV0}}
	clientConn, resp, err := d.Dial(b.TaskURL("t1"), nil)
	if resp != nil {
		_ = resp.Body.Close()
	}
	if err != nil {
		t.Fatalf("Dial(bridge) error = %v", err)
	}
	t.Cleanup(func() { _ = clientConn.Close() })

	{
		want := []byte("hello")
		if err := clientConn.WriteMessage(websocket.BinaryMessage, want); err != nil {
			t.Fatalf("WriteMessage(binary) error = %v", err)
		}

		_ = clientConn.SetReadDeadline(time.Now().Add(2 * time.Second))
		mt, got, err := clientConn.ReadMessage()
		if err != nil {
			t.Fatalf("ReadMessage(binary) error = %v", err)
		}
		if mt != websocket.BinaryMessage {
			t.Fatalf("ReadMessage(binary) messageType = %d, want %d", mt, websocket.BinaryMessage)
		}
		if string(got) != string(want) {
			t.Fatalf("ReadMessage(binary) payload = %q, want %q", string(got), string(want))
		}
	}

	{
		want := `{"op":"winsize","winsize":{"cols":80,"rows":24}}`
		if err := clientConn.WriteMessage(websocket.TextMessage, []byte(want)); err != nil {
			t.Fatalf("WriteMessage(text) error = %v", err)
		}

		_ = clientConn.SetReadDeadline(time.Now().Add(2 * time.Second))
		mt, got, err := clientConn.ReadMessage()
		if err != nil {
			t.Fatalf("ReadMessage(text) error = %v", err)
		}
		if mt != websocket.TextMessage {
			t.Fatalf("ReadMessage(text) messageType = %d, want %d", mt, websocket.TextMessage)
		}
		if string(got) != want {
			t.Fatalf("ReadMessage(text) payload = %q, want %q", string(got), want)
		}
	}
}
