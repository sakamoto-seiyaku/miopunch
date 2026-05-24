package task

import (
	"sync"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/pocstate"
)

type approveRuntime struct {
	mu sync.Mutex

	store *controlplane.InviteStore
	code  controlplane.InviteCodeV0

	inviteID string
	stateDir string
	selfID   pocstate.Identity
	netState pocstate.Net
	head     pocstate.GovernanceHeadSnapshotV1

	mailboxes []*mqttMailbox
	requests  map[string]approveRuntimeRequest
}

type approveRuntimeRequest struct {
	req controlplane.Message

	body         joinRequestBodyV0
	replyTopic   string
	memberPeerID string
	memberXPub   []byte
}

func (m *Manager) registerApproveRuntime(taskID string, rt *approveRuntime) {
	if m == nil || rt == nil {
		return
	}
	m.approvalMu.Lock()
	if m.approvalRuntimes == nil {
		m.approvalRuntimes = make(map[string]*approveRuntime)
	}
	m.approvalRuntimes[taskID] = rt
	m.approvalMu.Unlock()
}

func (m *Manager) unregisterApproveRuntime(taskID string) {
	if m == nil {
		return
	}
	m.approvalMu.Lock()
	delete(m.approvalRuntimes, taskID)
	m.approvalMu.Unlock()
}

func (m *Manager) approveRuntime(taskID string) (*approveRuntime, bool) {
	if m == nil {
		return nil, false
	}
	m.approvalMu.Lock()
	defer m.approvalMu.Unlock()
	rt, ok := m.approvalRuntimes[taskID]
	return rt, ok
}

func (rt *approveRuntime) upsertRequest(req approveRuntimeRequest) {
	if rt == nil {
		return
	}
	rt.mu.Lock()
	if rt.requests == nil {
		rt.requests = make(map[string]approveRuntimeRequest)
	}
	rt.requests[req.req.Route.MsgID] = req
	rt.mu.Unlock()
}

func (rt *approveRuntime) request(requestMsgID string) (approveRuntimeRequest, bool) {
	if rt == nil {
		return approveRuntimeRequest{}, false
	}
	rt.mu.Lock()
	defer rt.mu.Unlock()
	req, ok := rt.requests[requestMsgID]
	return req, ok
}
