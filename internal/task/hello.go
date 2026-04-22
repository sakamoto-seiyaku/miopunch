package task

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func (m *Manager) requireHello(ctx context.Context, taskID string, stream io.ReadWriteCloser) bool {
	if m == nil {
		return false
	}
	if stream == nil {
		m.addFact(taskID, poc.Fact{Message: "missing stream for hello handshake"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return false
	}

	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "state_dir: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return false
	}

	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure identity: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return false
	}

	approveDeclJSON, approveMsgID := findSelfApproveDeclJSON(stateDir, selfID.PeerID)

	sigB64, err := shellproto.SignHelloV0(selfID.Ed25519Priv, selfID.PeerID, approveMsgID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "sign hello: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return false
	}

	req := shellproto.Control{
		Op:     shellproto.OpHello,
		PeerID: selfID.PeerID,
		SigB64: sigB64,
	}
	if len(approveDeclJSON) > 0 {
		req.ApproveDecl = approveDeclJSON
	}

	helloCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()

	if err := shellproto.WriteJSON(stream, req); err != nil {
		m.addFact(taskID, poc.Fact{Message: "send hello: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return false
	}

	kind, payload, err := readFrameWithContext(helloCtx, stream)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "read hello response: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return false
	}
	if kind != shellproto.KindJSON {
		m.addFact(taskID, poc.Fact{Message: "unexpected hello response kind"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return false
	}

	var resp shellproto.Control
	if err := json.Unmarshal(payload, &resp); err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid hello response json: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return false
	}

	if strings.TrimSpace(resp.Op) != shellproto.OpHello || !resp.OK {
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
		return false
	}

	m.addFact(taskID, poc.Fact{Message: "hello=ok"})
	return true
}

func findSelfApproveDeclJSON(stateDir string, selfPeerID string) (json.RawMessage, string) {
	f, err := pocstate.LoadDecls(stateDir)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil, ""
		}
		return nil, ""
	}

	selfPeerID = strings.TrimSpace(selfPeerID)
	for _, d := range f.Decls {
		if strings.TrimSpace(d.Kind) != pocstate.DeclKindApproveMember {
			continue
		}
		var body pocstate.ApproveMemberBodyV0
		if err := json.Unmarshal(d.Body, &body); err != nil {
			continue
		}
		if strings.TrimSpace(body.MemberPeerID) != selfPeerID {
			continue
		}

		data, err := json.Marshal(d)
		if err != nil {
			continue
		}
		return data, d.MsgID
	}
	return nil, ""
}

func readFrameWithContext(ctx context.Context, stream io.ReadWriteCloser) (shellproto.Kind, []byte, error) {
	type frameResult struct {
		kind    shellproto.Kind
		payload []byte
		err     error
	}
	ch := make(chan frameResult, 1)
	go func() {
		kind, payload, err := shellproto.ReadFrame(stream)
		ch <- frameResult{kind: kind, payload: payload, err: err}
	}()

	select {
	case res := <-ch:
		return res.kind, res.payload, res.err
	case <-ctx.Done():
		_ = stream.Close()
		return 0, nil, ctx.Err()
	}
}
