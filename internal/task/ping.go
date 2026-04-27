package task

import (
	"context"
	"encoding/json"
	"strings"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func (m *Manager) runPingTask(taskID string, rawArgs []byte) {
	var args PingArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args = args.normalize()
	if args.PeerID == "" {
		m.addFact(taskID, poc.Fact{Message: "missing peer_id"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "use: miopunch ping <peer_id>"})
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

	open, ok := m.shellStreamOpen(taskID, shellproto.OpPing, "", "")
	if !ok {
		return
	}
	res, err := m.dialPeerStream(ctx, taskID, args.PeerID, cfg, open)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "dial peer: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	defer res.stream.Close()

	m.setStage(taskID, poc.StageCapabilityHandshake, "hello handshake")
	if !m.requirePeerStreamHello(ctx, taskID, res) {
		return
	}

	m.setStage(taskID, poc.StageCapabilityHandshake, "ping")

	req := shellproto.Control{Op: shellproto.OpPing}
	if err := shellproto.WriteJSON(res.stream, req); err != nil {
		m.addFact(taskID, poc.Fact{Message: "send ping: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	kind, payload, err := shellproto.ReadFrame(res.stream)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "read ping response: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	if kind != shellproto.KindJSON {
		m.addFact(taskID, poc.Fact{Message: "unexpected response kind"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	var resp shellproto.Control
	if err := json.Unmarshal(payload, &resp); err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid response json: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	if strings.TrimSpace(resp.Op) != shellproto.OpPing || !resp.OK {
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

	m.addFact(taskID, poc.Fact{Message: "ping=ok"})
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}
