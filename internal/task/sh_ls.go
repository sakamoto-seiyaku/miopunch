package task

import (
	"context"
	"encoding/json"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func (m *Manager) runShellListTask(taskID string, rawArgs []byte) {
	var args ShLSArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args = args.normalize()
	if args.PeerID == "" {
		m.addFact(taskID, poc.Fact{Message: "missing peer_id"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "use: miopunch sh ls <peer_id> [target]"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	ctx, cancel := context.WithTimeout(m.ctx, 60*time.Second)
	defer cancel()

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

	res, err := m.dialPeerStream(ctx, taskID, args.PeerID, cfg)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "dial peer: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	defer res.stream.Close()

	m.setStage(taskID, poc.StageCapabilityHandshake, "hello handshake")
	if !m.requireHello(ctx, taskID, res.stream) {
		return
	}

	m.setStage(taskID, poc.StageCapabilityHandshake, "shell list request")

	if err := shellproto.WriteJSON(res.stream, shellproto.Control{Op: shellproto.OpShLS, Target: args.Target}); err != nil {
		m.addFact(taskID, poc.Fact{Message: "send sh_ls: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	kind, payload, err := shellproto.ReadFrame(res.stream)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "read sh_ls response: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	if kind != shellproto.KindJSON {
		m.addFact(taskID, poc.Fact{Message: "unexpected sh_ls response kind"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	var resp shellproto.Control
	if err := json.Unmarshal(payload, &resp); err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid sh_ls response json: " + err.Error()})
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

	if args.Target == "" {
		for _, t := range resp.Targets {
			t = strings.TrimSpace(t)
			if t == "" {
				continue
			}
			m.addFact(taskID, poc.Fact{TermID: "target", Message: "target=" + t})
		}
		m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
		return
	}

	for _, s := range resp.Sessions {
		s = strings.TrimSpace(s)
		if s == "" {
			continue
		}
		m.addFact(taskID, poc.Fact{TermID: "session", Message: "session=" + s})
	}
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}
