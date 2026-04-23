package desktopbridge

import (
	"context"
	"crypto/rand"
	"encoding/base64"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

const ShellSubprotocolV0 = "miopunch.sh.v0"

type DialTaskWSFunc func(ctx context.Context, taskID string) (*websocket.Conn, error)

type TerminalWSBridge struct {
	token string
	addr  string

	dial DialTaskWSFunc

	ln  net.Listener
	srv *http.Server
}

func NewTerminalWSBridge(dial DialTaskWSFunc) (*TerminalWSBridge, error) {
	if dial == nil {
		return nil, errors.New("dial is nil")
	}

	token, err := randomToken(32)
	if err != nil {
		return nil, err
	}

	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return nil, err
	}

	b := &TerminalWSBridge{
		token: token,
		addr:  ln.Addr().String(),
		dial:  dial,
		ln:    ln,
	}

	mux := http.NewServeMux()
	mux.HandleFunc("/api/v0/tasks/", b.handleTaskWS)

	b.srv = &http.Server{
		Handler: mux,
	}

	go func() {
		_ = b.srv.Serve(ln)
	}()

	return b, nil
}

func (b *TerminalWSBridge) Addr() string {
	if b == nil {
		return ""
	}
	return b.addr
}

func (b *TerminalWSBridge) Token() string {
	if b == nil {
		return ""
	}
	return b.token
}

func (b *TerminalWSBridge) BaseURL() string {
	if b == nil || b.addr == "" {
		return ""
	}
	return "ws://" + b.addr
}

func (b *TerminalWSBridge) TaskURL(taskID string) string {
	if b == nil {
		return ""
	}
	return fmt.Sprintf("%s/api/v0/tasks/%s/ws?token=%s", b.BaseURL(), url.PathEscape(taskID), url.QueryEscape(b.token))
}

func (b *TerminalWSBridge) Close() error {
	if b == nil {
		return nil
	}
	err := b.srv.Close()
	_ = b.ln.Close()
	return err
}

func (b *TerminalWSBridge) handleTaskWS(w http.ResponseWriter, r *http.Request) {
	if !isLoopbackRemote(r.RemoteAddr) {
		http.Error(w, "loopback required", http.StatusForbidden)
		return
	}

	if strings.TrimSpace(r.URL.Query().Get("token")) != b.token {
		http.Error(w, "invalid token", http.StatusForbidden)
		return
	}

	taskID, ok := parseTaskIDFromPath(r.URL.Path)
	if !ok {
		http.NotFound(w, r)
		return
	}

	if !clientOffersSubprotocol(r, ShellSubprotocolV0) {
		http.Error(w, "missing required websocket subprotocol", http.StatusBadRequest)
		return
	}

	dialCtx, cancel := context.WithTimeout(r.Context(), 5*time.Second)
	defer cancel()

	backendConn, err := b.dial(dialCtx, taskID)
	if err != nil {
		http.Error(w, "failed to dial localapi websocket: "+err.Error(), http.StatusBadGateway)
		return
	}
	defer func() { _ = backendConn.Close() }()

	upgrader := websocket.Upgrader{
		Subprotocols: []string{ShellSubprotocolV0},
		CheckOrigin:  func(*http.Request) bool { return true },
	}
	frontConn, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		return
	}
	defer func() { _ = frontConn.Close() }()

	if frontConn.Subprotocol() != ShellSubprotocolV0 {
		_ = frontConn.WriteControl(
			websocket.CloseMessage,
			websocket.FormatCloseMessage(websocket.CloseProtocolError, "subprotocol required"),
			time.Now().Add(2*time.Second),
		)
		return
	}

	bridgeWebSockets(r.Context(), frontConn, backendConn)
}

func parseTaskIDFromPath(path string) (string, bool) {
	const prefix = "/api/v0/tasks/"
	const suffix = "/ws"

	if !strings.HasPrefix(path, prefix) || !strings.HasSuffix(path, suffix) {
		return "", false
	}
	taskID := strings.TrimSuffix(strings.TrimPrefix(path, prefix), suffix)
	taskID = strings.Trim(taskID, "/")
	if taskID == "" || strings.Contains(taskID, "/") {
		return "", false
	}
	return taskID, true
}

func clientOffersSubprotocol(r *http.Request, want string) bool {
	for _, p := range websocket.Subprotocols(r) {
		if strings.TrimSpace(p) == want {
			return true
		}
	}
	return false
}

func bridgeWebSockets(ctx context.Context, frontConn *websocket.Conn, backendConn *websocket.Conn) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	var closeOnce sync.Once
	closeAll := func() {
		closeOnce.Do(func() {
			cancel()
			_ = frontConn.Close()
			_ = backendConn.Close()
		})
	}

	var wg sync.WaitGroup

	wg.Add(1)
	go func() {
		defer wg.Done()
		defer closeAll()

		for {
			mt, payload, err := frontConn.ReadMessage()
			if err != nil {
				return
			}
			if err := backendConn.WriteMessage(mt, payload); err != nil {
				return
			}
		}
	}()

	wg.Add(1)
	go func() {
		defer wg.Done()
		defer closeAll()

		for {
			mt, payload, err := backendConn.ReadMessage()
			if err != nil {
				return
			}
			if err := frontConn.WriteMessage(mt, payload); err != nil {
				return
			}
		}
	}()

	wg.Wait()
	closeAll()
}

func isLoopbackRemote(remoteAddr string) bool {
	host, _, err := net.SplitHostPort(remoteAddr)
	if err != nil {
		return false
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

func randomToken(n int) (string, error) {
	if n <= 0 {
		return "", fmt.Errorf("invalid token length: %d", n)
	}
	buf := make([]byte, n)
	if _, err := rand.Read(buf); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(buf), nil
}
