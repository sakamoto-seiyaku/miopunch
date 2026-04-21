package task

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func (m *Manager) runShellAttachTask(taskID string, rawArgs []byte) {
	var args ShAttachArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args = args.normalize()
	if args.PeerID == "" {
		m.addFact(taskID, poc.Fact{Message: "missing peer_id"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "use: miopunch sh <peer_id> [target]"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	m.setStage(taskID, poc.StagePeerContact, "load peer config")
	cfg, ok, err := m.loadPeerConfig(args.PeerID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "load state: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	if !ok {
		m.addFact(taskID, poc.Fact{TermID: "peer_id", Message: "peer_id=" + args.PeerID})
		m.addSuggestion(taskID, poc.Suggestion{Message: "join first: miopunch join <code>"})
		m.done(taskID, poc.ReasonCodeNotFound, poc.ExitCodeNotFound)
		return
	}

	handshakeCtx, cancel := context.WithTimeout(m.ctx, 2*time.Minute)
	defer cancel()

	res, err := m.dialPeerStream(handshakeCtx, taskID, args.PeerID, cfg)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "dial peer: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	defer res.stream.Close()

	m.setStage(taskID, poc.StageCapabilityHandshake, "shell attach request")

	session := strings.TrimSpace(args.Session)
	if session == "" {
		session = "main"
	}
	if err := shellproto.WriteJSON(res.stream, shellproto.Control{
		Op:      shellproto.OpShAttach,
		Target:  args.Target,
		Session: session,
	}); err != nil {
		m.addFact(taskID, poc.Fact{Message: "send sh_attach: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	kind, payload, err := shellproto.ReadFrame(res.stream)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "read sh_attach response: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	if kind != shellproto.KindJSON {
		m.addFact(taskID, poc.Fact{Message: "unexpected sh_attach response kind"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	var resp shellproto.Control
	if err := json.Unmarshal(payload, &resp); err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid sh_attach response json: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	if !resp.OK {
		reason, exit := remoteReasonToPOC("")
		if resp.Error != nil {
			reason, exit = remoteReasonToPOC(resp.Error.ReasonCode)
			if strings.TrimSpace(resp.Error.Message) != "" {
				m.addFact(taskID, poc.Fact{Message: resp.Error.Message})
			}
			for _, s := range resp.Error.Suggestions {
				if strings.TrimSpace(s) != "" {
					m.addSuggestion(taskID, poc.Suggestion{Message: s})
				}
			}
		}
		m.done(taskID, reason, exit)
		return
	}

	if strings.TrimSpace(resp.Target) != "" {
		m.addFact(taskID, poc.Fact{TermID: "target", Message: "target=" + strings.TrimSpace(resp.Target)})
	}
	m.addFact(taskID, poc.Fact{TermID: "session", Message: "session=" + session})

	m.setStage(taskID, poc.StageSessionAttach, "waiting for local websocket")

	ws, err := m.awaitShellWS(taskID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry and attach within 30s"})
		m.done(taskID, poc.ReasonCodeTimeout, poc.ExitCodeTimeout)
		return
	}

	m.setStage(taskID, poc.StageSessionAttach, "shell websocket attached")

	_ = bridgeShell(m.ctx, ws, res.stream)
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}

func (m *Manager) awaitShellWS(taskID string) (*websocket.Conn, error) {
	m.mu.Lock()
	state := m.attachByTask[taskID]
	m.mu.Unlock()
	if state == nil || state.wsCh == nil {
		return nil, errors.New("missing sh_attach websocket waiter")
	}

	select {
	case conn := <-state.wsCh:
		if conn == nil {
			return nil, errors.New("websocket attach cancelled")
		}
		return conn, nil
	case <-time.After(30 * time.Second):
		return nil, errors.New("no websocket attach within 30s")
	}
}

type wsWrite struct {
	msgType int
	data    []byte

	control bool
}

func bridgeShell(ctx context.Context, ws *websocket.Conn, stream io.ReadWriteCloser) error {
	if ws == nil || stream == nil {
		return errors.New("missing websocket or stream")
	}

	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	var closeOnce sync.Once
	closeAll := func() {
		closeOnce.Do(func() {
			cancel()
			_ = ws.Close()
			_ = stream.Close()
		})
	}

	remoteWriteCh := make(chan struct {
		kind    shellproto.Kind
		payload []byte
	}, 32)

	wsWriteCh := make(chan wsWrite, 32)

	var wg sync.WaitGroup

	// WS -> remote
	wg.Add(1)
	go func() {
		defer wg.Done()
		defer closeAll()

		for {
			mt, payload, err := ws.ReadMessage()
			if err != nil {
				return
			}

			switch mt {
			case websocket.BinaryMessage:
				select {
				case remoteWriteCh <- struct {
					kind    shellproto.Kind
					payload []byte
				}{kind: shellproto.KindData, payload: payload}:
				case <-ctx.Done():
					return
				}
			case websocket.TextMessage:
				var ctl shellproto.Control
				if err := json.Unmarshal(payload, &ctl); err != nil {
					continue
				}
				if strings.TrimSpace(ctl.Op) != shellproto.OpWinSize {
					continue
				}
				data, _ := json.Marshal(ctl)
				select {
				case remoteWriteCh <- struct {
					kind    shellproto.Kind
					payload []byte
				}{kind: shellproto.KindJSON, payload: data}:
				case <-ctx.Done():
					return
				}
			default:
				continue
			}
		}
	}()

	// remote -> WS
	wg.Add(1)
	go func() {
		defer wg.Done()
		defer closeAll()

		for {
			kind, payload, err := shellproto.ReadFrame(stream)
			if err != nil {
				return
			}

			switch kind {
			case shellproto.KindData:
				select {
				case wsWriteCh <- wsWrite{msgType: websocket.BinaryMessage, data: payload}:
				case <-ctx.Done():
					return
				}
			case shellproto.KindJSON:
				var ctl shellproto.Control
				if err := json.Unmarshal(payload, &ctl); err != nil {
					continue
				}
				if strings.TrimSpace(ctl.Op) == shellproto.OpHeartbeat {
					continue
				}
			default:
				continue
			}
		}
	}()

	// Remote writer (single writer).
	wg.Add(1)
	go func() {
		defer wg.Done()
		defer closeAll()

		for {
			select {
			case <-ctx.Done():
				return
			case frame := <-remoteWriteCh:
				if err := shellproto.WriteFrame(stream, frame.kind, frame.payload); err != nil {
					return
				}
			}
		}
	}()

	// Heartbeat (visitor -> controlled).
	wg.Add(1)
	go func() {
		defer wg.Done()
		ticker := time.NewTicker(shellproto.DefaultHeartbeatInterval)
		defer ticker.Stop()

		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				data, _ := json.Marshal(shellproto.Control{Op: shellproto.OpHeartbeat})
				select {
				case remoteWriteCh <- struct {
					kind    shellproto.Kind
					payload []byte
				}{kind: shellproto.KindJSON, payload: data}:
				case <-ctx.Done():
					return
				}
			}
		}
	}()

	// WS writer (single writer).
	wg.Add(1)
	go func() {
		defer wg.Done()
		defer closeAll()

		for {
			select {
			case <-ctx.Done():
				return
			case w := <-wsWriteCh:
				if w.control {
					if err := ws.WriteControl(w.msgType, w.data, time.Now().Add(2*time.Second)); err != nil {
						return
					}
					continue
				}

				if err := ws.WriteMessage(w.msgType, w.data); err != nil {
					return
				}
			}
		}
	}()

	// WS ping loop (keepalive).
	wg.Add(1)
	go func() {
		defer wg.Done()
		ticker := time.NewTicker(shellproto.DefaultHeartbeatInterval)
		defer ticker.Stop()

		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				select {
				case wsWriteCh <- wsWrite{msgType: websocket.PingMessage, data: []byte("ping"), control: true}:
				case <-ctx.Done():
					return
				}
			}
		}
	}()

	wg.Wait()
	closeAll()
	return nil
}
