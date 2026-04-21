package localapi

import (
	"fmt"
	"net/http"
	"strings"
	"time"

	"github.com/gorilla/websocket"

	"github.com/miopunch/miopunch/internal/poc"
)

const shSubprotocolV0 = "miopunch.sh.v0"

func (s *Server) handleTaskWS(w http.ResponseWriter, r *http.Request) {
	rawTaskID := strings.TrimSpace(r.PathValue("task_id"))
	taskID, err := poc.CanonicalizeTaskID(rawTaskID)
	if err != nil {
		writeBadRequest(w, fmt.Sprintf("invalid task_id: %v", err))
		return
	}

	t, ok := s.tasks.Get(taskID)
	if !ok {
		writeNotFound(w, taskID, "task not found")
		return
	}
	if t.Kind != "sh_attach" {
		writeBadRequest(w, "websocket is only supported for sh_attach tasks")
		return
	}

	if !clientOffersSubprotocol(r, shSubprotocolV0) {
		reqID := taskID
		writeError(w, ErrorResponse{
			Stage:      "localapi",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Message:    "missing required websocket subprotocol",
			Facts: []poc.Fact{
				{TermID: "ws_subprotocol_want", Message: "want=" + shSubprotocolV0},
				{TermID: "ws_subprotocol_got", Message: "got=" + r.Header.Get("Sec-WebSocket-Protocol")},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry with header: Sec-WebSocket-Protocol: " + shSubprotocolV0},
			},
			RequestID: reqID,
		})
		return
	}

	upgrader := websocket.Upgrader{
		Subprotocols: []string{shSubprotocolV0},
		CheckOrigin: func(r *http.Request) bool {
			// LocalAPI is IPC-only; Host is already validated by middleware.
			return true
		},
	}
	conn, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		return
	}
	defer func() { _ = conn.Close() }()

	if conn.Subprotocol() != shSubprotocolV0 {
		_ = conn.WriteControl(
			websocket.CloseMessage,
			websocket.FormatCloseMessage(websocket.CloseProtocolError, "subprotocol required"),
			time.Now().Add(2*time.Second),
		)
		return
	}

	// Signal the task runtime so SSE consumers receive actionable output.
	_ = s.tasks.TriggerShellAttach(taskID)

	_ = conn.WriteControl(
		websocket.CloseMessage,
		websocket.FormatCloseMessage(websocket.CloseNormalClosure, "not implemented (POC-05)"),
		time.Now().Add(2*time.Second),
	)
}

func clientOffersSubprotocol(r *http.Request, want string) bool {
	for _, p := range websocket.Subprotocols(r) {
		if strings.TrimSpace(p) == want {
			return true
		}
	}
	return false
}
