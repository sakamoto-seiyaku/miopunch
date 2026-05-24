package task

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/poc"
)

const maintainNeighborsTimeout = 4 * time.Minute

func (m *Manager) runMaintainNeighborsTask(taskID string, rawArgs []byte) {
	var args MaintainNeighborsArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args = args.normalize()
	if strings.TrimSpace(args.P2PNetwork) != "" {
		if _, err := connectivity.ParseP2PNetwork(args.P2PNetwork); err != nil {
			m.addFact(taskID, poc.Fact{Message: err.Error()})
			m.addSuggestion(taskID, poc.Suggestion{Message: "use: --p2p-network auto|udp_only|tcp_only (or -u|-t)"})
			m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
			return
		}
	}

	ctx, cancel := context.WithTimeout(m.ctx, maintainNeighborsTimeout)
	defer cancel()

	m.setStage(taskID, poc.StagePeerContact, "select active neighbors")
	snap, err := m.TopologySnapshot()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "topology snapshot: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	active := make(map[string]struct{}, len(snap.Neighbors.Active))
	for _, edge := range snap.Neighbors.Active {
		peerID := strings.TrimSpace(edge.PeerID)
		if peerID != "" && edge.Healthy {
			active[peerID] = struct{}{}
		}
	}

	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("maintain_neighbors_selected=%d", len(snap.Neighbors.Selected))})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("maintain_neighbors_active_before=%d", len(active))})

	attempted := 0
	succeeded := 0
	failed := 0
	skippedActive := 0
	skippedUndialable := 0

	for _, selected := range snap.Neighbors.Selected {
		peerID := strings.TrimSpace(selected.PeerID)
		if peerID == "" {
			continue
		}
		if _, ok := active[peerID]; ok {
			skippedActive++
			continue
		}
		if !selected.Dialable {
			skippedUndialable++
			failed++
			m.addFact(taskID, poc.Fact{Message: "neighbor_skip_not_dialable=" + peerID})
			m.recordTopologyAttempt(TopologyAttempt{
				PeerID:        peerID,
				StartedAt:     time.Now().UTC().UnixMilli(),
				Outcome:       "fail",
				Stage:         string(poc.StagePeerContact),
				ReasonCode:    string(poc.ReasonCodeNotFound),
				StopCondition: "peer_config_missing",
			})
			continue
		}

		attempted++
		if m.runMaintainNeighborPing(ctx, taskID, peerID, args.P2PNetwork) {
			succeeded++
			active[peerID] = struct{}{}
			continue
		}
		failed++
	}

	post, err := m.TopologySnapshot()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "post topology snapshot: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("maintain_neighbors_attempted=%d", attempted)})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("maintain_neighbors_succeeded=%d", succeeded)})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("maintain_neighbors_failed=%d", failed)})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("maintain_neighbors_skipped_active=%d", skippedActive)})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("maintain_neighbors_skipped_not_dialable=%d", skippedUndialable)})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("active_neighbors=%d", len(post.Neighbors.Active))})
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}

func (m *Manager) runMaintainNeighborPing(ctx context.Context, taskID string, peerID string, p2pNetwork string) bool {
	args := PingArgs{PeerID: peerID, P2PNetwork: strings.TrimSpace(p2pNetwork)}
	raw, err := json.Marshal(args)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "marshal ping args: " + err.Error()})
		return false
	}
	child, err := m.CreateAndRun(CreateRequest{
		Kind: "ping",
		Args: raw,
	})
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "start neighbor ping: " + err.Error()})
		return false
	}
	m.addFact(taskID, poc.Fact{Message: "neighbor_ping_task=" + peerID + ":" + child.ID})

	final, err := m.waitTaskDone(ctx, child.ID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "wait neighbor ping: " + err.Error()})
		return false
	}
	if final.ExitCode == poc.ExitCodeOK {
		m.addFact(taskID, poc.Fact{Message: "neighbor_active=" + peerID})
		return true
	}
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("neighbor_failed=%s:%s", peerID, final.ReasonCode)})
	return false
}

func (m *Manager) waitTaskDone(ctx context.Context, taskID string) (Task, error) {
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	for {
		if t, ok := m.Get(taskID); ok && t.Status == StatusDone {
			return t, nil
		}
		select {
		case <-ctx.Done():
			return Task{}, ctx.Err()
		case <-ticker.C:
		}
	}
}
