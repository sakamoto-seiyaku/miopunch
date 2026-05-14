package task

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/logutil"
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

	if strings.TrimSpace(args.P2PNetwork) != "" {
		network, err := connectivity.ParseP2PNetwork(args.P2PNetwork)
		if err != nil {
			m.addFact(taskID, poc.Fact{Message: err.Error()})
			m.addSuggestion(taskID, poc.Suggestion{Message: "use: --p2p-network auto|udp_only|tcp_only (or -u/-t)"})
			m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
			return
		}
		cfg.P2PNetwork = string(network)
	}

	handshakeCtx, cancel := context.WithTimeout(m.ctx, 2*time.Minute)
	defer cancel()

	session := strings.TrimSpace(args.Session)
	if session == "" {
		session = "main"
	}

	open, ok := m.shellStreamOpen(taskID, shellproto.OpShAttach, args.Target, session)
	if !ok {
		return
	}
	res, err := m.dialPeerStream(handshakeCtx, taskID, args.PeerID, cfg, open)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "dial peer: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	defer res.stream.Close()

	m.setStage(taskID, poc.StageCapabilityHandshake, "hello handshake")
	if !m.requirePeerStreamHello(handshakeCtx, taskID, res) {
		return
	}

	m.setStage(taskID, poc.StageCapabilityHandshake, "shell attach request")

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

	bridgeResult := bridgeShell(
		m.ctx,
		taskID,
		strings.TrimSpace(args.PeerID),
		ws,
		res.stream,
		strings.TrimSpace(resp.Target),
		session,
	)
	for _, fact := range bridgeResult.facts {
		m.addFact(taskID, fact)
	}
	for _, suggestion := range bridgeResult.suggestions {
		m.addSuggestion(taskID, suggestion)
	}
	m.done(taskID, bridgeResult.reasonCode, bridgeResult.exitCode)
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
			m.setShellAttachable(taskID, false)
			return nil, errors.New("websocket attach cancelled")
		}
		return conn, nil
	case <-time.After(30 * time.Second):
		m.setShellAttachable(taskID, false)
		return nil, errors.New("no websocket attach within 30s")
	}
}

type wsWrite struct {
	msgType int
	data    []byte

	control bool
}

type shellBridgeResult struct {
	reasonCode  poc.ReasonCode
	exitCode    poc.ExitCode
	facts       []poc.Fact
	suggestions []poc.Suggestion
}

func bridgeShell(ctx context.Context, taskID string, peerID string, ws *websocket.Conn, stream io.ReadWriteCloser, target string, session string) shellBridgeResult {
	if ws == nil || stream == nil {
		return shellBridgeFailure(
			poc.ReasonCodeUnavailable,
			poc.ExitCodeUnavailable,
			"task_bridge",
			"missing websocket or stream",
			"retry",
		)
	}

	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	var (
		result    shellBridgeResult
		resultSet bool
	)

	var closeOnce sync.Once
	closeAll := func() {
		closeOnce.Do(func() {
			cancel()
			_ = ws.Close()
			_ = stream.Close()
		})
	}

	var resultOnce sync.Once
	setResult := func(next shellBridgeResult) {
		resultOnce.Do(func() {
			result = next
			resultSet = true
			closeAll()
		})
	}

	remoteWriteCh := make(chan struct {
		kind    shellproto.Kind
		payload []byte
	}, 1)

	wsWriteCh := make(chan wsWrite, 1)

	var wg sync.WaitGroup

	// WS -> remote
	wg.Add(1)
	go func() {
		defer wg.Done()

		loggedFirstLocalData := false
		loggedFirstLocalWinSize := false
		for {
			mt, payload, err := ws.ReadMessage()
			if err != nil {
				if ctx.Err() != nil {
					return
				}
				if isExpectedShellWebSocketClose(err) {
					logutil.Infof(
						"sh_attach bridge local websocket closed: %s err=%v",
						shellBridgeLogContext(taskID, peerID, target, session),
						err,
					)
					setResult(shellBridgeSuccess())
					return
				}
				logutil.Warnf(
					"sh_attach bridge local websocket closed abnormally: %s err=%v",
					shellBridgeLogContext(taskID, peerID, target, session),
					err,
				)
				setResult(shellBridgeFailure(
					poc.ReasonCodeUnavailable,
					poc.ExitCodeUnavailable,
					"localapi_ws",
					shellWebSocketCloseDetail(err),
					"retry",
				))
				return
			}

			switch mt {
			case websocket.BinaryMessage:
				if !loggedFirstLocalData {
					loggedFirstLocalData = true
					logutil.Infof(
						"sh_attach bridge first local websocket data: %s bytes=%d",
						shellBridgeLogContext(taskID, peerID, target, session),
						len(payload),
					)
				}
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
				if !loggedFirstLocalWinSize {
					loggedFirstLocalWinSize = true
					cols, rows := 0, 0
					if ctl.WinSize != nil {
						cols = ctl.WinSize.Cols
						rows = ctl.WinSize.Rows
					}
					logutil.Infof(
						"sh_attach bridge first local websocket winsize: %s bytes=%d size=%dx%d",
						shellBridgeLogContext(taskID, peerID, target, session),
						len(payload),
						cols,
						rows,
					)
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

		loggedFirstRemoteData := false
		loggedFirstRemoteJSON := false
		for {
			kind, payload, err := shellproto.ReadFrame(stream)
			if err != nil {
				if ctx.Err() != nil {
					return
				}
				logutil.Warnf(
					"sh_attach bridge remote stream closed: %s err=%v",
					shellBridgeLogContext(taskID, peerID, target, session),
					err,
				)
				setResult(shellBridgeFailure(
					poc.ReasonCodeSHConnectorFail,
					poc.ExitCodeUnavailable,
					"acceptor",
					"read shell stream: "+strings.TrimSpace(err.Error()),
					"retry",
				))
				return
			}

			switch kind {
			case shellproto.KindData:
				if !loggedFirstRemoteData {
					loggedFirstRemoteData = true
					logutil.Infof(
						"sh_attach bridge first remote stream data: %s bytes=%d",
						shellBridgeLogContext(taskID, peerID, target, session),
						len(payload),
					)
				}
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
				if !loggedFirstRemoteJSON {
					loggedFirstRemoteJSON = true
					logutil.Infof(
						"sh_attach bridge first remote stream json: %s bytes=%d op=%s ok=%t",
						shellBridgeLogContext(taskID, peerID, target, session),
						len(payload),
						strings.TrimSpace(ctl.Op),
						ctl.OK,
					)
				}
				switch strings.TrimSpace(ctl.Op) {
				case shellproto.OpHeartbeat:
					continue
				case shellproto.OpShAttach:
					if ctl.OK {
						continue
					}
					logutil.Warnf(
						"sh_attach bridge remote shell failure: %s reason_code=%s message=%s",
						shellBridgeLogContext(taskID, peerID, target, session),
						strings.TrimSpace(shellControlReasonCode(&ctl)),
						strings.TrimSpace(shellControlMessage(&ctl)),
					)
					setResult(shellBridgeResultFromRemoteControl(target, &ctl))
					return
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

		loggedFirstDataWrite := false
		loggedFirstJSONWrite := false
		for {
			select {
			case <-ctx.Done():
				return
			case frame := <-remoteWriteCh:
				switch frame.kind {
				case shellproto.KindData:
					if !loggedFirstDataWrite {
						loggedFirstDataWrite = true
						logutil.Infof(
							"sh_attach bridge first remote stream data write: %s bytes=%d",
							shellBridgeLogContext(taskID, peerID, target, session),
							len(frame.payload),
						)
					}
				case shellproto.KindJSON:
					if !loggedFirstJSONWrite {
						loggedFirstJSONWrite = true
						logutil.Infof(
							"sh_attach bridge first remote stream json write: %s bytes=%d",
							shellBridgeLogContext(taskID, peerID, target, session),
							len(frame.payload),
						)
					}
				}
				if err := shellproto.WriteFrame(stream, frame.kind, frame.payload); err != nil {
					if ctx.Err() != nil {
						return
					}
					logutil.Warnf(
						"sh_attach bridge remote write failed: %s err=%v",
						shellBridgeLogContext(taskID, peerID, target, session),
						err,
					)
					setResult(shellBridgeFailure(
						poc.ReasonCodeSHConnectorFail,
						poc.ExitCodeUnavailable,
						"acceptor",
						"write shell stream: "+strings.TrimSpace(err.Error()),
						"retry",
					))
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

		loggedFirstWSDataWrite := false
		for {
			select {
			case <-ctx.Done():
				return
			case w := <-wsWriteCh:
				if w.control {
					if err := ws.WriteControl(w.msgType, w.data, time.Now().Add(2*time.Second)); err != nil {
						if ctx.Err() != nil {
							return
						}
						logutil.Warnf(
							"sh_attach bridge local websocket control write failed: %s err=%v",
							shellBridgeLogContext(taskID, peerID, target, session),
							err,
						)
						setResult(shellBridgeFailure(
							poc.ReasonCodeUnavailable,
							poc.ExitCodeUnavailable,
							"localapi_ws",
							shellWebSocketCloseDetail(err),
							"retry",
						))
						return
					}
					continue
				}

				if w.msgType == websocket.BinaryMessage && !loggedFirstWSDataWrite {
					loggedFirstWSDataWrite = true
					logutil.Infof(
						"sh_attach bridge first local websocket data write: %s bytes=%d",
						shellBridgeLogContext(taskID, peerID, target, session),
						len(w.data),
					)
				}
				if err := ws.WriteMessage(w.msgType, w.data); err != nil {
					if ctx.Err() != nil {
						return
					}
					logutil.Warnf(
						"sh_attach bridge local websocket write failed: %s err=%v",
						shellBridgeLogContext(taskID, peerID, target, session),
						err,
					)
					setResult(shellBridgeFailure(
						poc.ReasonCodeUnavailable,
						poc.ExitCodeUnavailable,
						"localapi_ws",
						shellWebSocketCloseDetail(err),
						"retry",
					))
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
	if resultSet {
		return result
	}
	return shellBridgeFailure(
		poc.ReasonCodeUnavailable,
		poc.ExitCodeUnavailable,
		"task_bridge",
		"shell bridge ended without a close result",
		"retry",
	)
}

func shellBridgeSuccess() shellBridgeResult {
	return shellBridgeResult{
		reasonCode: poc.ReasonCodeOK,
		exitCode:   poc.ExitCodeOK,
	}
}

func shellBridgeFailure(reasonCode poc.ReasonCode, exitCode poc.ExitCode, layer string, detail string, suggestions ...string) shellBridgeResult {
	facts := make([]poc.Fact, 0, 2)
	layer = strings.TrimSpace(layer)
	detail = strings.TrimSpace(detail)
	if layer != "" {
		facts = append(facts, poc.Fact{TermID: "shell_layer", Message: "shell_layer=" + layer})
	}
	if detail != "" {
		facts = append(facts, poc.Fact{TermID: "shell_close", Message: "shell_close=" + detail})
	}

	outSuggestions := make([]poc.Suggestion, 0, len(suggestions))
	for _, suggestion := range suggestions {
		suggestion = strings.TrimSpace(suggestion)
		if suggestion == "" {
			continue
		}
		outSuggestions = append(outSuggestions, poc.Suggestion{Message: suggestion})
	}

	return shellBridgeResult{
		reasonCode:  reasonCode,
		exitCode:    exitCode,
		facts:       facts,
		suggestions: outSuggestions,
	}
}

func shellBridgeLogContext(taskID string, peerID string, target string, session string) string {
	return fmt.Sprintf(
		"task_id=%s peer_id=%s target=%s session=%s",
		strings.TrimSpace(taskID),
		strings.TrimSpace(peerID),
		strings.TrimSpace(target),
		strings.TrimSpace(session),
	)
}

func shellBridgeResultFromRemoteControl(target string, ctl *shellproto.Control) shellBridgeResult {
	if ctl == nil || ctl.Error == nil {
		return shellBridgeFailure(
			poc.ReasonCodeSHConnectorFail,
			poc.ExitCodeUnavailable,
			"acceptor",
			"remote shell attach failed without details",
			"retry",
		)
	}

	reason, exit := remoteReasonToPOC(ctl.Error.ReasonCode)
	if reason == poc.ReasonCodeUnavailable && strings.TrimSpace(target) != "" {
		reason = poc.ReasonCodeSHConnectorFail
	}

	layer := shellLayerFromMessage(ctl.Error.Message)
	if layer == "" {
		layer = shellLayerForTargetAndReason(target, ctl.Error.ReasonCode)
	}

	suggestions := ctl.Error.Suggestions
	if len(suggestions) == 0 {
		suggestions = []string{"retry"}
	}
	return shellBridgeFailure(reason, exit, layer, ctl.Error.Message, suggestions...)
}

func shellLayerFromMessage(message string) string {
	msg := strings.ToLower(strings.TrimSpace(message))
	switch {
	case strings.HasPrefix(msg, "ssh "):
		return "ssh"
	case strings.HasPrefix(msg, "wsl "):
		return "wsl"
	case strings.HasPrefix(msg, "tmux "):
		return "tmux"
	case strings.HasPrefix(msg, "pty "):
		return "pty"
	case strings.HasPrefix(msg, "acceptor "):
		return "acceptor"
	case strings.HasPrefix(msg, "local websocket "):
		return "localapi_ws"
	default:
		return ""
	}
}

func shellLayerForTargetAndReason(target string, reason string) string {
	switch strings.TrimSpace(reason) {
	case "SH_TMUX_ATTACH_FAIL":
		if strings.TrimSpace(target) == "local" {
			return "tmux"
		}
	}

	switch {
	case strings.HasPrefix(strings.TrimSpace(target), "ssh:"):
		return "ssh"
	case strings.HasPrefix(strings.TrimSpace(target), "wsl:"):
		return "wsl"
	case strings.TrimSpace(target) == "local":
		return "pty"
	default:
		return "acceptor"
	}
}

func isExpectedShellWebSocketClose(err error) bool {
	if err == nil {
		return false
	}
	return websocket.IsCloseError(err, websocket.CloseNormalClosure, websocket.CloseGoingAway) || errors.Is(err, io.EOF)
}

func shellWebSocketCloseDetail(err error) string {
	if err == nil {
		return "local websocket closed"
	}

	var closeErr *websocket.CloseError
	if errors.As(err, &closeErr) {
		reason := strings.TrimSpace(closeErr.Text)
		if reason == "" {
			return fmt.Sprintf("local websocket closed (%d)", closeErr.Code)
		}
		return fmt.Sprintf("local websocket closed (%d): %s", closeErr.Code, reason)
	}

	return "local websocket closed: " + strings.TrimSpace(err.Error())
}

func shellControlReasonCode(ctl *shellproto.Control) string {
	if ctl == nil || ctl.Error == nil {
		return ""
	}
	return ctl.Error.ReasonCode
}

func shellControlMessage(ctl *shellproto.Control) string {
	if ctl == nil || ctl.Error == nil {
		return ""
	}
	return ctl.Error.Message
}
