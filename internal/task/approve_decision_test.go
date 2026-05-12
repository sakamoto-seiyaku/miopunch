package task

import (
	"encoding/base64"
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

func TestRunApproveDecisionTaskApprovesPersistedRequestAfterRestart(t *testing.T) {
	statePath, approveTaskID, requestMsgID := recordPersistedApprovalDecisionForTest(t)

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	raw, err := json.Marshal(ApproveDecisionArgs{
		ApproveTaskID: approveTaskID,
		RequestMsgID:  requestMsgID,
		Decision:      controlplane.ApprovalDecisionApprove,
	})
	if err != nil {
		t.Fatalf("json.Marshal(ApproveDecisionArgs) error = %v", err)
	}
	created, err := m.CreateAndRun(CreateRequest{Kind: "approve_decision", Args: raw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(approve_decision) error = %v", err)
	}
	final := waitTaskDoneForTest(t, m, created.ID)
	if final.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("approve_decision ReasonCode = %q, want %q; facts=%v", final.ReasonCode, poc.ReasonCodeOK, final.Facts)
	}

	store := approvalStoreForTaskTest(t, statePath)
	lookup, err := store.LookupApprovalRequest(approveTaskID, requestMsgID)
	if err != nil {
		t.Fatalf("LookupApprovalRequest() error = %v", err)
	}
	if lookup.Request.Status != controlplane.ApprovalStatusApproved {
		t.Fatalf("LookupApprovalRequest().Request.Status = %q, want %q", lookup.Request.Status, controlplane.ApprovalStatusApproved)
	}
}

func TestRunApproveDecisionTaskRejectsPersistedRequestAfterRestart(t *testing.T) {
	statePath, approveTaskID, requestMsgID := recordPersistedApprovalDecisionForTest(t)

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	raw, err := json.Marshal(ApproveDecisionArgs{
		ApproveTaskID: approveTaskID,
		RequestMsgID:  requestMsgID,
		Decision:      controlplane.ApprovalDecisionReject,
	})
	if err != nil {
		t.Fatalf("json.Marshal(ApproveDecisionArgs) error = %v", err)
	}
	created, err := m.CreateAndRun(CreateRequest{Kind: "approve_decision", Args: raw})
	if err != nil {
		t.Fatalf("Manager.CreateAndRun(approve_decision) error = %v", err)
	}
	final := waitTaskDoneForTest(t, m, created.ID)
	if final.ReasonCode != poc.ReasonCodeOK {
		t.Fatalf("approve_decision ReasonCode = %q, want %q; facts=%v", final.ReasonCode, poc.ReasonCodeOK, final.Facts)
	}

	store := approvalStoreForTaskTest(t, statePath)
	lookup, err := store.LookupApprovalRequest(approveTaskID, requestMsgID)
	if err != nil {
		t.Fatalf("LookupApprovalRequest() error = %v", err)
	}
	if lookup.Request.Status != controlplane.ApprovalStatusRejected {
		t.Fatalf("LookupApprovalRequest().Request.Status = %q, want %q", lookup.Request.Status, controlplane.ApprovalStatusRejected)
	}
}

func recordPersistedApprovalDecisionForTest(t *testing.T) (string, string, string) {
	t.Helper()

	broker := startTCPMQTTBroker(t)
	statePath := filepath.Join(t.TempDir(), "state.json")
	saveLocalStateForInviteTest(t, statePath, broker)
	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	if _, err := pocstate.EnsureIdentity(stateDir); err != nil {
		t.Fatalf("pocstate.EnsureIdentity(%q) error = %v", stateDir, err)
	}

	memberID, err := pocstate.EnsureIdentity(t.TempDir())
	if err != nil {
		t.Fatalf("pocstate.EnsureIdentity(member) error = %v", err)
	}
	requestMsgID, err := controlplane.NewMsgID()
	if err != nil {
		t.Fatalf("controlplane.NewMsgID() error = %v", err)
	}
	requestMsgID, err = controlplane.CanonicalizeMsgID(requestMsgID)
	if err != nil {
		t.Fatalf("controlplane.CanonicalizeMsgID(%q) error = %v", requestMsgID, err)
	}

	body := joinRequestBodyV0{
		ReplyTopic:    "miopunch/test/reply",
		MemberName:    "Restart laptop",
		PlatformHint:  "linux",
		Ed25519PubB64: memberID.Ed25519PubB64(),
		X25519PubB64:  memberID.X25519PubB64(),
	}
	bodyJSON, err := json.Marshal(body)
	if err != nil {
		t.Fatalf("json.Marshal(joinRequestBodyV0) error = %v", err)
	}

	approveTaskID := "task-approve-restart"
	store := approvalStoreForTaskTest(t, statePath)
	if _, _, err := store.RecordApprovalRequest("miopunch/test/restart-invite", time.Now().UTC().Add(time.Hour).UnixMilli(), 1, controlplane.ApprovalRequestRecord{
		ApproveTaskID: approveTaskID,
		RequestMsgID:  requestMsgID,
		MemberPeerID:  memberID.PeerID,
		MemberName:    body.MemberName,
		PlatformHint:  body.PlatformHint,
		DecisionMaterial: &controlplane.ApprovalDecisionMaterial{
			InviteBrokers:                   []string{broker},
			ReplyTopic:                      body.ReplyTopic,
			JoinRequestBodyB64URL:           base64.RawURLEncoding.EncodeToString(bodyJSON),
			MemberEd25519PubB64:             body.Ed25519PubB64,
			MemberX25519PubB64:              body.X25519PubB64,
			ValidatedAtUnixMs:               time.Now().UTC().UnixMilli(),
			ValidatedRequestExpiresAtUnixMs: time.Now().UTC().Add(time.Hour).UnixMilli(),
			ValidatedRequestSenderID:        memberID.PeerID,
		},
	}); err != nil {
		t.Fatalf("RecordApprovalRequest() error = %v", err)
	}
	return statePath, approveTaskID, requestMsgID
}

func approvalStoreForTaskTest(t *testing.T, statePath string) *controlplane.InviteStore {
	t.Helper()

	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		t.Fatalf("pocstate.StateDir(%q) error = %v", statePath, err)
	}
	store, err := controlplane.NewInviteStore(stateDir)
	if err != nil {
		t.Fatalf("controlplane.NewInviteStore(%q) error = %v", stateDir, err)
	}
	return store
}
