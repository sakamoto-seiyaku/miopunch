package localapi

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/shellproto"
	"github.com/miopunch/miopunch/internal/task"
)

func TestTaskWS_ShellAttachLifecycle_NoNetwork(t *testing.T) {
	t.Parallel()

	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Peers: map[string]pocstate.PeerConfig{
			"peer1": {},
		},
	}); err != nil {
		t.Fatalf("save state: %v", err)
	}

	mgr := task.NewManagerWithStatePath(statePath)
	t.Cleanup(mgr.Close)
	mgr.SetDialPeerStreamHook(func(ctx context.Context, taskID string, peerID string, cfg pocstate.PeerConfig) (io.ReadWriteCloser, error) {
		clientConn, serverConn := net.Pipe()

		go func() {
			defer serverConn.Close()

			var hello shellproto.Control
			if err := shellproto.ReadJSON(serverConn, &hello); err != nil {
				return
			}

			if strings.TrimSpace(hello.Op) != shellproto.OpHello {
				_ = shellproto.WriteJSON(serverConn, shellproto.Control{
					Op: shellproto.OpHello,
					OK: false,
					Error: &shellproto.ControlError{
						ReasonCode: shellproto.ReasonHelloRequired,
						Message:    "unexpected op",
					},
				})
				return
			}

			_ = shellproto.WriteJSON(serverConn, shellproto.Control{
				Op: shellproto.OpHello,
				OK: true,
			})

			var req shellproto.Control
			if err := shellproto.ReadJSON(serverConn, &req); err != nil {
				return
			}

			if strings.TrimSpace(req.Op) != shellproto.OpShAttach {
				_ = shellproto.WriteJSON(serverConn, shellproto.Control{
					Op: shellproto.OpShAttach,
					OK: false,
					Error: &shellproto.ControlError{
						ReasonCode: "SH_CONNECTOR_FAIL",
						Message:    "unexpected op",
					},
				})
				return
			}

			_ = shellproto.WriteJSON(serverConn, shellproto.Control{
				Op:     shellproto.OpShAttach,
				OK:     true,
				Target: strings.TrimSpace(req.Target),
			})

			for {
				kind, payload, err := shellproto.ReadFrame(serverConn)
				if err != nil {
					return
				}
				if kind != shellproto.KindData {
					continue
				}
				if err := shellproto.WriteFrame(serverConn, shellproto.KindData, payload); err != nil {
					return
				}
			}
		}()

		return clientConn, nil
	})

	srv := NewServer(ListenModeUser, mgr)
	ts := httptest.NewServer(srv.Handler())
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	t.Cleanup(cancel)

	body, err := json.Marshal(map[string]any{
		"kind": "sh_attach",
		"args": map[string]any{
			"peer_id": "peer1",
			"target":  "local",
			"session": "main",
		},
	})
	if err != nil {
		t.Fatalf("marshal create task body: %v", err)
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, ts.URL+"/api/v0/tasks", bytes.NewReader(body))
	if err != nil {
		t.Fatalf("new create task request: %v", err)
	}
	req.Host = poc.LocalAPIHost
	req.Header.Set("Content-Type", "application/json")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("create task request: %v", err)
	}
	t.Cleanup(func() { _ = resp.Body.Close() })
	if resp.StatusCode != http.StatusCreated {
		b, _ := io.ReadAll(resp.Body)
		t.Fatalf("POST /api/v0/tasks status = %d, want %d, body=%s", resp.StatusCode, http.StatusCreated, b)
	}

	var created task.Task
	if err := json.NewDecoder(resp.Body).Decode(&created); err != nil {
		t.Fatalf("decode create task response: %v", err)
	}
	if created.Kind != "sh_attach" {
		t.Fatalf("created task kind = %q, want %q", created.Kind, "sh_attach")
	}

	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http") + "/api/v0/tasks/" + created.ID + "/ws"
	dialer := websocket.Dialer{
		Subprotocols: []string{shSubprotocolV0},
	}
	ws, _, err := dialer.DialContext(ctx, wsURL, http.Header{
		"Host": []string{poc.LocalAPIHost},
	})
	if err != nil {
		t.Fatalf("DialContext(%q) error = %v, want nil", wsURL, err)
	}
	t.Cleanup(func() { _ = ws.Close() })
	if ws.Subprotocol() != shSubprotocolV0 {
		t.Fatalf("ws.Subprotocol() = %q, want %q", ws.Subprotocol(), shSubprotocolV0)
	}

	want := []byte("hello")
	if err := ws.WriteMessage(websocket.BinaryMessage, want); err != nil {
		t.Fatalf("ws.WriteMessage(BinaryMessage) error = %v, want nil", err)
	}

	_ = ws.SetReadDeadline(time.Now().Add(2 * time.Second))
	msgType, got, err := ws.ReadMessage()
	if err != nil {
		t.Fatalf("ws.ReadMessage() error = %v, want nil", err)
	}
	if msgType != websocket.BinaryMessage {
		t.Fatalf("ws.ReadMessage() type = %d, want %d", msgType, websocket.BinaryMessage)
	}
	if !bytes.Equal(got, want) {
		t.Fatalf("ws.ReadMessage() payload = %q, want %q", got, want)
	}

	_ = ws.WriteControl(
		websocket.CloseMessage,
		websocket.FormatCloseMessage(websocket.CloseNormalClosure, "bye"),
		time.Now().Add(2*time.Second),
	)
	_ = ws.Close()

	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		tk, ok := mgr.Get(created.ID)
		if ok && tk.Status == task.StatusDone {
			if tk.ReasonCode != poc.ReasonCodeOK {
				t.Fatalf("task reason_code = %q, want %q", tk.ReasonCode, poc.ReasonCodeOK)
			}
			if tk.ExitCode != poc.ExitCodeOK {
				t.Fatalf("task exit_code = %d, want %d", tk.ExitCode, poc.ExitCodeOK)
			}
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("task did not reach done status within deadline")
}

func TestTaskWS_ShellAttachLateRemoteFailure_NoNetwork(t *testing.T) {
	t.Parallel()

	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Peers: map[string]pocstate.PeerConfig{
			"peer1": {},
		},
	}); err != nil {
		t.Fatalf("save state: %v", err)
	}

	mgr := task.NewManagerWithStatePath(statePath)
	t.Cleanup(mgr.Close)
	mgr.SetDialPeerStreamHook(func(ctx context.Context, taskID string, peerID string, cfg pocstate.PeerConfig) (io.ReadWriteCloser, error) {
		clientConn, serverConn := net.Pipe()

		go func() {
			defer serverConn.Close()

			var hello shellproto.Control
			if err := shellproto.ReadJSON(serverConn, &hello); err != nil {
				return
			}
			if strings.TrimSpace(hello.Op) != shellproto.OpHello {
				return
			}
			_ = shellproto.WriteJSON(serverConn, shellproto.Control{
				Op: shellproto.OpHello,
				OK: true,
			})

			var req shellproto.Control
			if err := shellproto.ReadJSON(serverConn, &req); err != nil {
				return
			}
			if strings.TrimSpace(req.Op) != shellproto.OpShAttach {
				return
			}
			session := strings.TrimSpace(req.Session)
			if session == "" {
				session = "main"
			}
			_ = shellproto.WriteJSON(serverConn, shellproto.Control{
				Op:      shellproto.OpShAttach,
				OK:      true,
				Target:  strings.TrimSpace(req.Target),
				Session: session,
			})

			if _, _, err := shellproto.ReadFrame(serverConn); err != nil {
				return
			}
			_ = shellproto.WriteJSON(serverConn, shellproto.Control{
				Op: shellproto.OpShAttach,
				OK: false,
				Error: &shellproto.ControlError{
					ReasonCode:  "SH_CONNECTOR_FAIL",
					Message:     "ssh process exited: process exited: 255",
					Suggestions: []string{"retry"},
				},
			})
		}()

		return clientConn, nil
	})

	srv := NewServer(ListenModeUser, mgr)
	ts := httptest.NewServer(srv.Handler())
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	t.Cleanup(cancel)

	body, err := json.Marshal(map[string]any{
		"kind": "sh_attach",
		"args": map[string]any{
			"peer_id": "peer1",
			"target":  "ssh:ops",
			"session": "main",
		},
	})
	if err != nil {
		t.Fatalf("marshal create task body: %v", err)
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, ts.URL+"/api/v0/tasks", bytes.NewReader(body))
	if err != nil {
		t.Fatalf("new create task request: %v", err)
	}
	req.Host = poc.LocalAPIHost
	req.Header.Set("Content-Type", "application/json")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("create task request: %v", err)
	}
	t.Cleanup(func() { _ = resp.Body.Close() })
	if resp.StatusCode != http.StatusCreated {
		b, _ := io.ReadAll(resp.Body)
		t.Fatalf("POST /api/v0/tasks status = %d, want %d, body=%s", resp.StatusCode, http.StatusCreated, b)
	}

	var created task.Task
	if err := json.NewDecoder(resp.Body).Decode(&created); err != nil {
		t.Fatalf("decode create task response: %v", err)
	}

	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http") + "/api/v0/tasks/" + created.ID + "/ws"
	dialer := websocket.Dialer{
		Subprotocols: []string{shSubprotocolV0},
	}
	ws, _, err := dialer.DialContext(ctx, wsURL, http.Header{
		"Host": []string{poc.LocalAPIHost},
	})
	if err != nil {
		t.Fatalf("DialContext(%q) error = %v, want nil", wsURL, err)
	}
	t.Cleanup(func() { _ = ws.Close() })

	if err := ws.WriteMessage(websocket.BinaryMessage, []byte("hello")); err != nil {
		t.Fatalf("ws.WriteMessage(BinaryMessage) error = %v, want nil", err)
	}
	_ = ws.SetReadDeadline(time.Now().Add(2 * time.Second))
	_, _, _ = ws.ReadMessage()

	tk := waitShellTaskDone(t, mgr, created.ID, 2*time.Second)
	if tk.Stage != poc.StageSessionAttach {
		t.Fatalf("task stage = %q, want %q", tk.Stage, poc.StageSessionAttach)
	}
	if tk.ReasonCode != poc.ReasonCodeSHConnectorFail {
		t.Fatalf("task reason_code = %q, want %q", tk.ReasonCode, poc.ReasonCodeSHConnectorFail)
	}
	if tk.ExitCode != poc.ExitCodeUnavailable {
		t.Fatalf("task exit_code = %d, want %d", tk.ExitCode, poc.ExitCodeUnavailable)
	}
	for _, want := range []string{
		"peer_id=peer1",
		"target=ssh:ops",
		"session=main",
		"shell_layer=ssh",
		"shell_close=ssh process exited: process exited: 255",
	} {
		if !taskHasFactSubstring(tk, want) {
			t.Fatalf("task facts = %#v, want substring %q", tk.Facts, want)
		}
	}
	if !taskHasSuggestionSubstring(tk, "retry") {
		t.Fatalf("task suggestions = %#v, want retry", tk.Suggestions)
	}
}

func waitShellTaskDone(t *testing.T, mgr *task.Manager, taskID string, timeout time.Duration) task.Task {
	t.Helper()

	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		tk, ok := mgr.Get(taskID)
		if ok && tk.Status == task.StatusDone {
			return tk
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("Manager.Get(%q) did not reach done", taskID)
	return task.Task{}
}

func taskHasFactSubstring(tk task.Task, want string) bool {
	for _, fact := range tk.Facts {
		if strings.Contains(strings.TrimSpace(fact.Message), want) {
			return true
		}
	}
	return false
}

func taskHasSuggestionSubstring(tk task.Task, want string) bool {
	for _, suggestion := range tk.Suggestions {
		if strings.Contains(strings.TrimSpace(suggestion.Message), want) {
			return true
		}
	}
	return false
}
