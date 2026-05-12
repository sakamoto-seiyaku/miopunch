package task

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

func (m *Manager) runApproveDecisionTask(taskID string, rawArgs []byte) {
	var args ApproveDecisionArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args = args.normalize()
	if args.ApproveTaskID == "" {
		m.addFact(taskID, poc.Fact{Message: "missing approve_task_id"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	if args.RequestMsgID == "" {
		m.addFact(taskID, poc.Fact{Message: "missing request_msg_id"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	requestMsgID, err := controlplane.CanonicalizeMsgID(args.RequestMsgID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid request_msg_id: " + err.Error()})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args.RequestMsgID = requestMsgID
	if args.Decision != controlplane.ApprovalDecisionApprove && args.Decision != controlplane.ApprovalDecisionReject {
		m.addFact(taskID, poc.Fact{Message: "invalid decision: " + args.Decision})
		m.addSuggestion(taskID, poc.Suggestion{Message: "use decision=approve or decision=reject"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	m.setStage(taskID, poc.StageCapabilityHandshake, "apply approval decision")

	stateDir, selfID, store, lookup, pending, err := m.loadApprovalDecisionRequest(args.ApproveTaskID, args.RequestMsgID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "load approval request failed: " + err.Error()})
		reason, exit := approvalDecisionFailureCode(err)
		m.done(taskID, reason, exit)
		return
	}

	ct, hit, rec, err := store.ResolveApprovalDecision(
		lookup.InviteTopic,
		lookup.ExpiresAtUnixMs,
		lookup.MaxUses,
		args.ApproveTaskID,
		args.RequestMsgID,
		args.Decision,
		func() ([]byte, error) {
			if args.Decision == controlplane.ApprovalDecisionReject {
				return sealApprovalRejection(selfID, lookup.InviteTopic, pending)
			}
			netState, head, err := m.prepareApprovalDecisionState(stateDir, selfID, pending.inviteBrokers)
			if err != nil {
				return nil, err
			}
			return m.buildMembershipBundleCiphertext(stateDir, selfID, netState, head, controlplane.InviteCodeV0{
				InviteTopic: lookup.InviteTopic,
			}, validatedJoinRequest{
				body:         pending.body,
				replyTopic:   pending.replyTopic,
				memberPeerID: pending.memberPeerID,
				memberXPub:   append([]byte(nil), pending.memberXPub...),
			})
		},
	)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "approval decision failed: " + err.Error()})
		reason, exit := approvalDecisionFailureCode(err)
		m.done(taskID, reason, exit)
		return
	}

	m.publishDesktopApprovalRequestsChange()

	pubCtx, cancel := context.WithTimeout(m.ctx, 10*time.Second)
	mbs, brokerFailures, err := openMQTTMailboxes(pubCtx, pending.inviteBrokers, "miopunch-approve-decision")
	if err != nil {
		cancel()
		m.addFact(taskID, poc.Fact{Message: "mqtt connect failed: " + err.Error()})
		m.addMQTTBrokerFailures(taskID, brokerFailures)
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	m.addMQTTBrokerFailures(taskID, brokerFailures)
	defer closeMQTTMailboxes(mbs)

	pubErr := publishMQTTAny(pubCtx, mbs, pending.replyTopic, ct)
	cancel()
	if pubErr != nil {
		m.addFact(taskID, poc.Fact{Message: "publish approval response failed: " + pubErr.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	m.addFact(taskID, poc.Fact{TermID: "approve_task_id", Message: "approve_task_id=" + args.ApproveTaskID})
	m.addFact(taskID, poc.Fact{TermID: "request_msg_id", Message: "request_msg_id=" + args.RequestMsgID})
	m.addFact(taskID, poc.Fact{TermID: "member_peer_id", Message: "member_peer_id=" + rec.MemberPeerID})
	m.addFact(taskID, poc.Fact{Message: "decision=" + args.Decision})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("idempotency_hit=%v", hit)})
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}

func approvalDecisionFailureCode(err error) (poc.ReasonCode, poc.ExitCode) {
	switch {
	case errors.Is(err, controlplane.ErrApprovalDecisionInvalid):
		return poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest
	case errors.Is(err, controlplane.ErrApprovalRequestNotFound):
		return poc.ReasonCodeNotFound, poc.ExitCodeNotFound
	case errors.Is(err, controlplane.ErrApprovalDecisionConflict):
		return poc.ReasonCodeConflict, poc.ExitCodeConflict
	case errors.Is(err, controlplane.ErrInviteExpired):
		return poc.ReasonCodeTimeout, poc.ExitCodeTimeout
	case errors.Is(err, controlplane.ErrInviteUsesExhausted):
		return poc.ReasonCodeConflict, poc.ExitCodeConflict
	case errors.Is(err, controlplane.ErrApprovalMaterialNotCached):
		return poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable
	default:
		return poc.ReasonCodeInternal, poc.ExitCodeInternal
	}
}

type approvalDecisionRequest struct {
	body          joinRequestBodyV0
	replyTopic    string
	memberPeerID  string
	memberXPub    []byte
	inviteBrokers []string
}

func (m *Manager) loadApprovalDecisionRequest(approveTaskID string, requestMsgID string) (string, pocstate.Identity, *controlplane.InviteStore, controlplane.ApprovalRequestLookup, approvalDecisionRequest, error) {
	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		return "", pocstate.Identity{}, nil, controlplane.ApprovalRequestLookup{}, approvalDecisionRequest{}, err
	}
	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		return "", pocstate.Identity{}, nil, controlplane.ApprovalRequestLookup{}, approvalDecisionRequest{}, fmt.Errorf("ensure identity: %w", err)
	}
	store, err := controlplane.NewInviteStore(stateDir)
	if err != nil {
		return "", pocstate.Identity{}, nil, controlplane.ApprovalRequestLookup{}, approvalDecisionRequest{}, err
	}
	lookup, err := store.LookupApprovalRequest(approveTaskID, requestMsgID)
	if err != nil {
		return "", pocstate.Identity{}, nil, controlplane.ApprovalRequestLookup{}, approvalDecisionRequest{}, err
	}
	pending, err := approvalDecisionRequestFromLookup(lookup)
	if err != nil {
		return "", pocstate.Identity{}, nil, controlplane.ApprovalRequestLookup{}, approvalDecisionRequest{}, err
	}
	return stateDir, selfID, store, lookup, pending, nil
}

func approvalDecisionRequestFromLookup(lookup controlplane.ApprovalRequestLookup) (approvalDecisionRequest, error) {
	material := lookup.Request.DecisionMaterial
	if material == nil {
		return approvalDecisionRequest{}, controlplane.ErrApprovalMaterialNotCached
	}
	bodyB64 := strings.TrimSpace(material.JoinRequestBodyB64URL)
	if bodyB64 == "" {
		return approvalDecisionRequest{}, controlplane.ErrApprovalMaterialNotCached
	}
	bodyJSON, err := base64.RawURLEncoding.DecodeString(bodyB64)
	if err != nil {
		return approvalDecisionRequest{}, fmt.Errorf("decode join request body: %w", err)
	}
	var body joinRequestBodyV0
	if err := json.Unmarshal(bodyJSON, &body); err != nil {
		return approvalDecisionRequest{}, fmt.Errorf("unmarshal join request body: %w", err)
	}
	if strings.TrimSpace(body.Ed25519PubB64) == "" {
		body.Ed25519PubB64 = strings.TrimSpace(material.MemberEd25519PubB64)
	}
	if strings.TrimSpace(body.X25519PubB64) == "" {
		body.X25519PubB64 = strings.TrimSpace(material.MemberX25519PubB64)
	}
	if strings.TrimSpace(body.ReplyTopic) == "" {
		body.ReplyTopic = strings.TrimSpace(material.ReplyTopic)
	}
	replyTopic := strings.TrimSpace(body.ReplyTopic)
	if replyTopic == "" {
		return approvalDecisionRequest{}, controlplane.ErrApprovalMaterialNotCached
	}
	memberXPub, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(body.X25519PubB64))
	if err != nil || len(memberXPub) != 32 {
		return approvalDecisionRequest{}, controlplane.ErrApprovalMaterialNotCached
	}
	inviteBrokers := normalizeBrokerCandidates(material.InviteBrokers)
	if len(inviteBrokers) == 0 {
		return approvalDecisionRequest{}, controlplane.ErrApprovalMaterialNotCached
	}

	return approvalDecisionRequest{
		body:          body,
		replyTopic:    replyTopic,
		memberPeerID:  strings.TrimSpace(lookup.Request.MemberPeerID),
		memberXPub:    memberXPub,
		inviteBrokers: inviteBrokers,
	}, nil
}

func (m *Manager) prepareApprovalDecisionState(stateDir string, selfID pocstate.Identity, inviteBrokers []string) (pocstate.Net, pocstate.GovernanceHeadSnapshotV1, error) {
	netState, err := pocstate.EnsureNet(stateDir, inviteBrokers)
	if err != nil {
		return pocstate.Net{}, pocstate.GovernanceHeadSnapshotV1{}, fmt.Errorf("ensure net: %w", err)
	}
	head, err := pocstate.EnsureGovernanceHeadSnapshot(stateDir, netState.NetID, selfID)
	if err != nil {
		return pocstate.Net{}, pocstate.GovernanceHeadSnapshotV1{}, fmt.Errorf("ensure head snapshot: %w", err)
	}
	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		return pocstate.Net{}, pocstate.GovernanceHeadSnapshotV1{}, fmt.Errorf("ensure decls: %w", err)
	}
	st, err := m.loadState()
	if err != nil {
		return pocstate.Net{}, pocstate.GovernanceHeadSnapshotV1{}, fmt.Errorf("load state: %w", err)
	}
	if st.Local == nil {
		st.Local = &pocstate.LocalConfig{}
	}
	st.Local.NormalizeDefaults()
	st.Local.PeerID = selfID.PeerID
	if strings.TrimSpace(st.Local.ProxyName) == "" {
		st.Local.ProxyName = selfID.PeerID
	}
	if strings.TrimSpace(st.Local.SecretKey) == "" {
		secretKey, _, err := newSecretKeyB64URLNoPad()
		if err != nil {
			return pocstate.Net{}, pocstate.GovernanceHeadSnapshotV1{}, fmt.Errorf("new secret_key: %w", err)
		}
		st.Local.SecretKey = secretKey
	}
	st.EnsureLocalDefaults()
	st.Local.SetMQTTBrokers(netState.BrokersEffective)
	if err := m.saveState(st); err != nil {
		return pocstate.Net{}, pocstate.GovernanceHeadSnapshotV1{}, fmt.Errorf("save state: %w", err)
	}
	return netState, head, nil
}

func sealApprovalRejection(selfID pocstate.Identity, inviteTopic string, pending approvalDecisionRequest) ([]byte, error) {
	if strings.TrimSpace(pending.memberPeerID) == "" {
		return nil, errors.New("empty member peer id")
	}
	pt, err := json.Marshal(approvalRejectionV0{
		Status: controlplane.ApprovalStatusRejected,
		Reason: "approval rejected",
	})
	if err != nil {
		return nil, fmt.Errorf("marshal approval rejection: %w", err)
	}
	return controlplane.SealInviteMembershipBundleV0(
		selfID.X25519Priv,
		pending.memberXPub,
		inviteTopic,
		selfID.PeerID,
		pending.memberPeerID,
		pt,
	)
}

func isApprovalDecisionTask(taskObj Task) bool {
	return strings.TrimSpace(taskObj.Kind) == "approve_decision"
}
