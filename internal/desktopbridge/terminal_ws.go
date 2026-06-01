package desktopbridge

import (
	"context"
	"crypto/rand"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"

	"github.com/miopunch/miopunch/internal/logutil"
)

const ShellSubprotocolV0 = "miopunch.sh.v0"

type ShellStream interface {
	io.ReadWriteCloser
}

type DialShellFunc func(ctx context.Context, shellSessionID string) (ShellStream, error)

type TerminalWSBridge struct {
	token string
	addr  string

	dial DialShellFunc

	ln  net.Listener
	srv *http.Server
}

func NewTerminalWSBridge(dial DialShellFunc) (*TerminalWSBridge, error) {
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
	mux.HandleFunc("/api/v1/shell/", b.handleShellWS)

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

func (b *TerminalWSBridge) ShellURL(shellSessionID string) string {
	if b == nil {
		return ""
	}
	return fmt.Sprintf(
		"%s/api/v1/shell/%s/ws?token=%s",
		b.BaseURL(),
		url.PathEscape(shellSessionID),
		url.QueryEscape(b.token),
	)
}

func (b *TerminalWSBridge) Close() error {
	if b == nil {
		return nil
	}
	err := b.srv.Close()
	_ = b.ln.Close()
	return err
}

func (b *TerminalWSBridge) handleShellWS(w http.ResponseWriter, r *http.Request) {
	if !isLoopbackRemote(r.RemoteAddr) {
		http.Error(w, "loopback required", http.StatusForbidden)
		return
	}

	if strings.TrimSpace(r.URL.Query().Get("token")) != b.token {
		http.Error(w, "invalid token", http.StatusForbidden)
		return
	}

	shellSessionID, ok := parseShellSessionIDFromPath(r.URL.Path)
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

	backendStream, err := b.dial(dialCtx, shellSessionID)
	if err != nil {
		logutil.Warnf("terminal bridge backend dial failed: shell_session_id=%s err=%v", shellSessionID, err)
		http.Error(w, "failed to dial localapi shell stream: "+err.Error(), http.StatusBadGateway)
		return
	}
	defer func() { _ = backendStream.Close() }()
	logutil.Infof("terminal bridge backend attached: shell_session_id=%s", shellSessionID)

	upgrader := websocket.Upgrader{
		Subprotocols: []string{ShellSubprotocolV0},
		CheckOrigin:  func(*http.Request) bool { return true },
	}
	frontConn, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		logutil.Warnf("terminal bridge frontend websocket upgrade failed: shell_session_id=%s err=%v", shellSessionID, err)
		return
	}
	defer func() { _ = frontConn.Close() }()
	logutil.Infof("terminal bridge frontend websocket attached: shell_session_id=%s remote=%s", shellSessionID, r.RemoteAddr)

	if frontConn.Subprotocol() != ShellSubprotocolV0 {
		_ = frontConn.WriteControl(
			websocket.CloseMessage,
			websocket.FormatCloseMessage(websocket.CloseProtocolError, "subprotocol required"),
			time.Now().Add(2*time.Second),
		)
		return
	}

	bridgeShellStream(r.Context(), shellSessionID, frontConn, backendStream)
}

func parseShellSessionIDFromPath(path string) (string, bool) {
	const prefix = "/api/v1/shell/"
	const suffix = "/ws"

	if !strings.HasPrefix(path, prefix) || !strings.HasSuffix(path, suffix) {
		return "", false
	}
	shellSessionID := strings.TrimSuffix(strings.TrimPrefix(path, prefix), suffix)
	shellSessionID = strings.Trim(shellSessionID, "/")
	if shellSessionID == "" || strings.Contains(shellSessionID, "/") {
		return "", false
	}
	return shellSessionID, true
}

func clientOffersSubprotocol(r *http.Request, want string) bool {
	for _, p := range websocket.Subprotocols(r) {
		if strings.TrimSpace(p) == want {
			return true
		}
	}
	return false
}

func bridgeShellStream(ctx context.Context, shellSessionID string, frontConn *websocket.Conn, backendStream ShellStream) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	var closeOnce sync.Once
	closeAll := func() {
		closeOnce.Do(func() {
			cancel()
			_ = frontConn.Close()
			_ = backendStream.Close()
		})
	}

	var wg sync.WaitGroup

	wg.Add(1)
	go func() {
		defer wg.Done()
		defer closeAll()

		loggedFirstFrontToBackend := false
		for {
			mt, payload, err := frontConn.ReadMessage()
			if err != nil {
				logutil.Infof("terminal bridge frontend read closed: shell_session_id=%s err=%v", shellSessionID, err)
				return
			}
			if mt != websocket.BinaryMessage && mt != websocket.TextMessage {
				continue
			}
			if !loggedFirstFrontToBackend {
				loggedFirstFrontToBackend = true
				logutil.Infof(
					"terminal bridge first frontend to backend frame: shell_session_id=%s message_type=%d bytes=%d",
					shellSessionID,
					mt,
					len(payload),
				)
			}
			if len(payload) == 0 {
				continue
			}
			if _, err := backendStream.Write(payload); err != nil {
				logutil.Warnf("terminal bridge backend write failed: shell_session_id=%s err=%v", shellSessionID, err)
				return
			}
		}
	}()

	wg.Add(1)
	go func() {
		defer wg.Done()
		defer closeAll()

		buf := make([]byte, 32*1024)
		loggedFirstBackendToFront := false
		for {
			n, err := backendStream.Read(buf)
			if n > 0 {
				payload := append([]byte(nil), buf[:n]...)
				if !loggedFirstBackendToFront {
					loggedFirstBackendToFront = true
					logutil.Infof(
						"terminal bridge first backend to frontend frame: shell_session_id=%s bytes=%d",
						shellSessionID,
						len(payload),
					)
				}
				if writeErr := frontConn.WriteMessage(websocket.BinaryMessage, payload); writeErr != nil {
					logutil.Warnf("terminal bridge frontend write failed: shell_session_id=%s err=%v", shellSessionID, writeErr)
					return
				}
			}
			if err != nil {
				if cleanShellStreamClose(err) {
					writeFrontendClose(frontConn, websocket.CloseNormalClosure, "shell exited")
				} else {
					logutil.Infof("terminal bridge backend read closed: shell_session_id=%s err=%v", shellSessionID, err)
				}
				return
			}
		}
	}()

	wg.Wait()
	closeAll()
}

func cleanShellStreamClose(err error) bool {
	return err == nil || errors.Is(err, io.EOF) || errors.Is(err, net.ErrClosed)
}

func writeFrontendClose(frontConn *websocket.Conn, code int, reason string) {
	_ = frontConn.WriteControl(
		websocket.CloseMessage,
		websocket.FormatCloseMessage(code, reason),
		time.Now().Add(2*time.Second),
	)
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
