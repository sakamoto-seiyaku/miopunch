package controlplane

import (
	"errors"
	"path/filepath"
	"testing"
	"time"
)

func TestInviteStoreRecordApprovalRequestCoalescesDuplicates(t *testing.T) {
	store, inviteTopic, expiresAt := newApprovalInviteStoreForTest(t)
	requestMsgID := newMsgIDForApprovalTest(t)

	rec, created, err := store.RecordApprovalRequest(inviteTopic, expiresAt, 2, ApprovalRequestRecord{
		ApproveTaskID: "task-approve",
		RequestMsgID:  requestMsgID,
		MemberPeerID:  "peer-a",
		MemberName:    "Laptop",
		PlatformHint:  "linux",
	})
	if err != nil {
		t.Fatalf("RecordApprovalRequest(first) error = %v", err)
	}
	if !created {
		t.Fatalf("RecordApprovalRequest(first) created = false, want true")
	}
	if rec.Status != ApprovalStatusPending {
		t.Fatalf("RecordApprovalRequest(first).Status = %q, want %q", rec.Status, ApprovalStatusPending)
	}

	rec, created, err = store.RecordApprovalRequest(inviteTopic, expiresAt, 2, ApprovalRequestRecord{
		ApproveTaskID: "task-approve",
		RequestMsgID:  requestMsgID,
		MemberPeerID:  "peer-a",
		MemberName:    "Laptop updated",
		PlatformHint:  "linux",
		V4Hint:        "easy",
	})
	if err != nil {
		t.Fatalf("RecordApprovalRequest(duplicate) error = %v", err)
	}
	if created {
		t.Fatalf("RecordApprovalRequest(duplicate) created = true, want false")
	}
	if rec.MemberName != "Laptop updated" {
		t.Errorf("RecordApprovalRequest(duplicate).MemberName = %q, want updated value", rec.MemberName)
	}

	inviteID, err := InviteIDFromTopic(inviteTopic)
	if err != nil {
		t.Fatalf("InviteIDFromTopic() error = %v", err)
	}
	stored, err := store.loadInvite(inviteID)
	if err != nil {
		t.Fatalf("loadInvite() error = %v", err)
	}
	if got := len(stored.ApprovalRequests); got != 1 {
		t.Fatalf("stored approval request count = %d, want 1", got)
	}
	if stored.UsesLeft != 2 {
		t.Fatalf("stored UsesLeft = %d, want 2", stored.UsesLeft)
	}
}

func TestInviteStoreLookupApprovalRequestIncludesDecisionMaterialAndListRedacts(t *testing.T) {
	store, inviteTopic, expiresAt := newApprovalInviteStoreForTest(t)
	requestMsgID := newMsgIDForApprovalTest(t)

	if _, _, err := store.RecordApprovalRequest(inviteTopic, expiresAt, 1, ApprovalRequestRecord{
		ApproveTaskID: "task-approve",
		RequestMsgID:  requestMsgID,
		MemberPeerID:  "peer-a",
		DecisionMaterial: &ApprovalDecisionMaterial{
			InviteBrokers:         []string{"127.0.0.1:1883"},
			ReplyTopic:            "miopunch/test/reply",
			JoinRequestBodyB64URL: "eyJ0ZXN0Ijp0cnVlfQ",
			MemberX25519PubB64:    "member-x25519",
		},
	}); err != nil {
		t.Fatalf("RecordApprovalRequest() error = %v", err)
	}

	lookup, err := store.LookupApprovalRequest("task-approve", requestMsgID)
	if err != nil {
		t.Fatalf("LookupApprovalRequest() error = %v", err)
	}
	if lookup.InviteTopic != inviteTopic {
		t.Fatalf("LookupApprovalRequest().InviteTopic = %q, want %q", lookup.InviteTopic, inviteTopic)
	}
	if lookup.Request.DecisionMaterial == nil {
		t.Fatalf("LookupApprovalRequest().Request.DecisionMaterial = nil, want persisted material")
	}
	if lookup.Request.DecisionMaterial.ReplyTopic != "miopunch/test/reply" {
		t.Fatalf("LookupApprovalRequest().Request.DecisionMaterial.ReplyTopic = %q, want miopunch/test/reply", lookup.Request.DecisionMaterial.ReplyTopic)
	}

	listed, err := store.ListApprovalRequests()
	if err != nil {
		t.Fatalf("ListApprovalRequests() error = %v", err)
	}
	if len(listed) != 1 {
		t.Fatalf("ListApprovalRequests() length = %d, want 1", len(listed))
	}
	if listed[0].DecisionMaterial != nil {
		t.Fatalf("ListApprovalRequests()[0].DecisionMaterial = %#v, want nil", listed[0].DecisionMaterial)
	}
	if listed[0].ResponseCTB64URL != "" {
		t.Fatalf("ListApprovalRequests()[0].ResponseCTB64URL = %q, want empty", listed[0].ResponseCTB64URL)
	}
}

func TestInviteStoreResolveApprovalDecisionApproveIsIdempotent(t *testing.T) {
	store, inviteTopic, expiresAt := newApprovalInviteStoreForTest(t)
	requestMsgID := newMsgIDForApprovalTest(t)
	recordApprovalForTest(t, store, inviteTopic, expiresAt, requestMsgID)

	ct, hit, rec, err := store.ResolveApprovalDecision(inviteTopic, expiresAt, 1, "task-approve", requestMsgID, ApprovalDecisionApprove, func() ([]byte, error) {
		return []byte("approved-response"), nil
	})
	if err != nil {
		t.Fatalf("ResolveApprovalDecision(approve) error = %v", err)
	}
	if hit {
		t.Fatalf("ResolveApprovalDecision(approve) hit = true, want false")
	}
	if string(ct) != "approved-response" {
		t.Fatalf("ResolveApprovalDecision(approve) ct = %q, want approved-response", ct)
	}
	if rec.Status != ApprovalStatusApproved {
		t.Fatalf("ResolveApprovalDecision(approve).Status = %q, want %q", rec.Status, ApprovalStatusApproved)
	}

	ct, hit, rec, err = store.ResolveApprovalDecision(inviteTopic, expiresAt, 1, "task-approve", requestMsgID, ApprovalDecisionApprove, func() ([]byte, error) {
		return []byte("should-not-run"), nil
	})
	if err != nil {
		t.Fatalf("ResolveApprovalDecision(approve duplicate) error = %v", err)
	}
	if !hit {
		t.Fatalf("ResolveApprovalDecision(approve duplicate) hit = false, want true")
	}
	if string(ct) != "approved-response" {
		t.Fatalf("ResolveApprovalDecision(approve duplicate) ct = %q, want cached response", ct)
	}

	inviteID, err := InviteIDFromTopic(inviteTopic)
	if err != nil {
		t.Fatalf("InviteIDFromTopic() error = %v", err)
	}
	stored, err := store.loadInvite(inviteID)
	if err != nil {
		t.Fatalf("loadInvite() error = %v", err)
	}
	if stored.UsesLeft != 0 {
		t.Fatalf("stored UsesLeft = %d, want 0", stored.UsesLeft)
	}
	if _, ok := stored.HandledRequests[requestMsgID]; !ok {
		t.Fatalf("stored HandledRequests missing %q", requestMsgID)
	}
	if rec.Decision != ApprovalDecisionApprove {
		t.Fatalf("ResolveApprovalDecision(approve duplicate).Decision = %q, want approve", rec.Decision)
	}
}

func TestInviteStoreResolveApprovalDecisionRejectDoesNotConsumeUses(t *testing.T) {
	store, inviteTopic, expiresAt := newApprovalInviteStoreForTest(t)
	requestMsgID := newMsgIDForApprovalTest(t)
	recordApprovalForTest(t, store, inviteTopic, expiresAt, requestMsgID)

	_, hit, rec, err := store.ResolveApprovalDecision(inviteTopic, expiresAt, 1, "task-approve", requestMsgID, ApprovalDecisionReject, func() ([]byte, error) {
		return []byte("rejected-response"), nil
	})
	if err != nil {
		t.Fatalf("ResolveApprovalDecision(reject) error = %v", err)
	}
	if hit {
		t.Fatalf("ResolveApprovalDecision(reject) hit = true, want false")
	}
	if rec.Status != ApprovalStatusRejected {
		t.Fatalf("ResolveApprovalDecision(reject).Status = %q, want %q", rec.Status, ApprovalStatusRejected)
	}

	_, hit, _, err = store.ResolveApprovalDecision(inviteTopic, expiresAt, 1, "task-approve", requestMsgID, ApprovalDecisionReject, func() ([]byte, error) {
		return []byte("should-not-run"), nil
	})
	if err != nil {
		t.Fatalf("ResolveApprovalDecision(reject duplicate) error = %v", err)
	}
	if !hit {
		t.Fatalf("ResolveApprovalDecision(reject duplicate) hit = false, want true")
	}

	inviteID, err := InviteIDFromTopic(inviteTopic)
	if err != nil {
		t.Fatalf("InviteIDFromTopic() error = %v", err)
	}
	stored, err := store.loadInvite(inviteID)
	if err != nil {
		t.Fatalf("loadInvite() error = %v", err)
	}
	if stored.UsesLeft != 1 {
		t.Fatalf("stored UsesLeft = %d, want 1", stored.UsesLeft)
	}
	if _, ok := stored.HandledRequests[requestMsgID]; ok {
		t.Fatalf("stored HandledRequests[%q] exists after rejection, want absent", requestMsgID)
	}
}

func TestInviteStoreResolveApprovalDecisionRejectsConflicts(t *testing.T) {
	store, inviteTopic, expiresAt := newApprovalInviteStoreForTest(t)
	requestMsgID := newMsgIDForApprovalTest(t)
	recordApprovalForTest(t, store, inviteTopic, expiresAt, requestMsgID)

	if _, _, _, err := store.ResolveApprovalDecision(inviteTopic, expiresAt, 1, "task-approve", requestMsgID, ApprovalDecisionReject, func() ([]byte, error) {
		return []byte("rejected-response"), nil
	}); err != nil {
		t.Fatalf("ResolveApprovalDecision(reject) error = %v", err)
	}

	_, _, _, err := store.ResolveApprovalDecision(inviteTopic, expiresAt, 1, "task-approve", requestMsgID, ApprovalDecisionApprove, func() ([]byte, error) {
		return []byte("should-not-run"), nil
	})
	if !errors.Is(err, ErrApprovalDecisionConflict) {
		t.Fatalf("ResolveApprovalDecision(conflict) error = %v, want ErrApprovalDecisionConflict", err)
	}
}

func newApprovalInviteStoreForTest(t *testing.T) (*InviteStore, string, int64) {
	t.Helper()

	store, err := NewInviteStore(filepath.Join(t.TempDir(), "state"))
	if err != nil {
		t.Fatalf("NewInviteStore() error = %v", err)
	}
	return store, "miopunch/test/invite", time.Now().UTC().Add(time.Hour).UnixMilli()
}

func newMsgIDForApprovalTest(t *testing.T) string {
	t.Helper()

	msgID, err := NewMsgID()
	if err != nil {
		t.Fatalf("NewMsgID() error = %v", err)
	}
	return msgID
}

func recordApprovalForTest(t *testing.T, store *InviteStore, inviteTopic string, expiresAt int64, requestMsgID string) {
	t.Helper()

	if _, _, err := store.RecordApprovalRequest(inviteTopic, expiresAt, 1, ApprovalRequestRecord{
		ApproveTaskID: "task-approve",
		RequestMsgID:  requestMsgID,
		MemberPeerID:  "peer-a",
	}); err != nil {
		t.Fatalf("RecordApprovalRequest() error = %v", err)
	}
}
