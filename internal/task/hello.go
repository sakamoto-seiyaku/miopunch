package task

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	"strings"
	"time"

	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func (m *Manager) shellStreamOpen(taskID string, op string, target string, session string) (dataplane.StreamOpen, bool) {
	if m == nil {
		return dataplane.StreamOpen{}, false
	}

	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "state_dir: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return dataplane.StreamOpen{}, false
	}

	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure identity: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return dataplane.StreamOpen{}, false
	}

	approveDeclJSON, approveMsgID := findSelfApproveDeclJSON(stateDir, selfID.PeerID)
	sigB64, err := shellproto.SignHelloV0(selfID.Ed25519Priv, selfID.PeerID, approveMsgID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "sign hello: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return dataplane.StreamOpen{}, false
	}

	metadata := map[string]string{
		"peer_id": selfID.PeerID,
		"sig_b64": sigB64,
		"op":      strings.TrimSpace(op),
	}
	if len(approveDeclJSON) > 0 {
		metadata["approve_decl"] = string(approveDeclJSON)
	}
	if target = strings.TrimSpace(target); target != "" {
		metadata["target"] = target
	}
	if session = strings.TrimSpace(session); session != "" {
		metadata["session"] = session
	}

	return dataplane.StreamOpen{
		Kind:     dataplane.StreamKindShellV0,
		Metadata: metadata,
	}, true
}

func (m *Manager) requirePeerStreamHello(ctx context.Context, taskID string, res *dialResult) bool {
	if res == nil || res.stream == nil {
		m.addFact(taskID, poc.Fact{Message: "missing stream for hello handshake"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return false
	}
	if res.legacyHello {
		return m.requireHello(ctx, taskID, res.stream)
	}
	return m.waitStreamOpenHello(ctx, taskID, res.stream)
}

func (m *Manager) waitStreamOpenHello(ctx context.Context, taskID string, stream io.ReadWriteCloser) bool {
	helloCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()

	kind, payload, err := readFrameWithContext(helloCtx, stream)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "read stream-open hello response: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return false
	}
	if kind != shellproto.KindJSON {
		m.addFact(taskID, poc.Fact{Message: "unexpected stream-open hello response kind"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return false
	}

	var resp shellproto.Control
	if err := json.Unmarshal(payload, &resp); err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid stream-open hello response json: " + err.Error()})
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
