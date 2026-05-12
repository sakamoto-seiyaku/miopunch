package task

import (
	"context"
	"crypto/ed25519"
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

type validatedJoinRequest struct {
	req          controlplane.Message
	body         joinRequestBodyV0
	replyTopic   string
	memberPeerID string
	memberXPub   []byte
}

type approvalRejectionV0 struct {
	Status string `json:"status"`
	Reason string `json:"reason,omitempty"`
}

func (m *Manager) runApproveTask(taskID string, rawArgs []byte) {
	var args ApproveArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args = args.normalize()
	if args.Code == "" {
		m.addFact(taskID, poc.Fact{Message: "missing invite code"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "use: miopunch approve <invite_code-or-url>"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	m.setStage(taskID, poc.StageControlPlaneReady, "decode invite code")
	code, err := controlplane.DecodeInviteCodeV0(args.Code)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid invite code: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "verify the code and retry"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "state_dir: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure identity: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	if selfID.PeerID != code.IssuerPeerID {
		m.addFact(taskID, poc.Fact{Message: "invite code issuer mismatch"})
		m.addFact(taskID, poc.Fact{Message: "invite_issuer_peer_id=" + code.IssuerPeerID})
		m.addFact(taskID, poc.Fact{Message: "self_peer_id=" + selfID.PeerID})
		m.addSuggestion(taskID, poc.Suggestion{Message: "run approve on the issuing/admin machine that generated the invite"})
		m.done(taskID, poc.ReasonCodeForbidden, poc.ExitCodeForbidden)
		return
	}
	if strings.TrimSpace(code.IssuerEd25519PubB64) != strings.TrimSpace(selfID.Ed25519PubB64()) {
		m.addFact(taskID, poc.Fact{Message: "invite code issuer ed25519 pubkey mismatch"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry approve on the issuing machine"})
		m.done(taskID, poc.ReasonCodeForbidden, poc.ExitCodeForbidden)
		return
	}
	if strings.TrimSpace(code.IssuerX25519PubB64) != strings.TrimSpace(selfID.X25519PubB64()) {
		m.addFact(taskID, poc.Fact{Message: "invite code issuer x25519 pubkey mismatch"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry approve on the issuing machine"})
		m.done(taskID, poc.ReasonCodeForbidden, poc.ExitCodeForbidden)
		return
	}

	inviteSecret, err := decodeInviteSecretB64(code.InviteSecretB64)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid invite_secret_b64"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	// Approve lifetime is bounded by the invite expiry.
	expiresAt := time.UnixMilli(code.ExpiresAtUnixMs).UTC()
	ctx, cancel := context.WithDeadline(m.ctx, expiresAt)
	defer cancel()

	m.setStage(taskID, poc.StageSelfDiscovery, "ensure governance state")

	store, err := controlplane.NewInviteStore(stateDir)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "invite store: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	idem, err := controlplane.NewInviteIdempotency(store, nil)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "invite idempotency: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	inviteID, err := store.EnsureInvite(code.InviteTopic, code.ExpiresAtUnixMs, code.MaxUses)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure invite record: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	netState, err := pocstate.EnsureNet(stateDir, code.InviteBrokers)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure net: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	head, err := pocstate.EnsureGovernanceHeadSnapshot(stateDir, netState.NetID, selfID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure head snapshot: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure decls: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	st, err := m.loadState()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "load state: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
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
			m.addFact(taskID, poc.Fact{Message: "new secret_key: " + err.Error()})
			m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
			m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
			return
		}
		st.Local.SecretKey = secretKey
	}
	st.EnsureLocalDefaults()
	st.Local.SetMQTTBrokers(netState.BrokersEffective)

	if err := m.saveState(st); err != nil {
		m.addFact(taskID, poc.Fact{Message: "save state: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	m.setStage(taskID, poc.StagePeerContact, "connect invite brokers")
	m.addFact(taskID, poc.Fact{TermID: "invite_brokers", Message: "invite_brokers=" + strings.Join(code.InviteBrokers, ",")})

	mbs, brokerFailures, err := openMQTTMailboxes(ctx, code.InviteBrokers, "miopunch-invite-approve")
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "mqtt connect failed: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "verify broker reachability and retry"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "set local.mqtt_broker to a reachable broker shared by both machines"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	for _, failure := range brokerFailures {
		m.addFact(taskID, poc.Fact{Message: "mqtt broker skipped: " + failure})
	}

	subCtx, cancelSub := context.WithTimeout(ctx, 10*time.Second)
	defer cancelSub()
	mbs, brokerFailures, err = subscribeMQTTMailboxes(subCtx, mbs, code.InviteTopic)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "subscribe invite_topic failed: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	for _, failure := range brokerFailures {
		m.addFact(taskID, poc.Fact{Message: "mqtt broker skipped: " + failure})
	}
	defer closeMQTTMailboxes(mbs)

	evCh, stop := fanInMailboxEvents(ctx, mbs)
	defer stop()

	if args.ExplicitReview {
		rt := &approveRuntime{
			store:     store,
			code:      code,
			inviteID:  inviteID,
			stateDir:  stateDir,
			selfID:    selfID,
			netState:  netState,
			head:      head,
			mailboxes: mbs,
			requests:  make(map[string]approveRuntimeRequest),
		}
		m.registerApproveRuntime(taskID, rt)
		defer m.unregisterApproveRuntime(taskID)
		m.addFact(taskID, poc.Fact{TermID: "explicit_review", Message: "explicit_review=true"})
	}

	m.addFact(taskID, poc.Fact{TermID: "peer_id", Message: "peer_id=" + selfID.PeerID})
	m.addFact(taskID, poc.Fact{TermID: "net_id", Message: "net_id=" + netState.NetID})
	m.addFact(taskID, poc.Fact{TermID: "invite_id", Message: "invite_id=" + inviteID})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("invite_max_uses=%d", code.MaxUses)})
	m.addSuggestion(taskID, poc.Suggestion{Message: "wait for join_request on another machine: miopunch join <invite_code>"})

	m.setStage(taskID, poc.StageCapabilityHandshake, "wait join_request")

	approvedUnique := 0
	for {
		select {
		case <-ctx.Done():
			if args.ExplicitReview {
				if expired, err := store.ExpireApprovalRequestsForTask(code.InviteTopic, code.ExpiresAtUnixMs, code.MaxUses, taskID); err != nil {
					m.addFact(taskID, poc.Fact{Message: "expire approval requests failed: " + err.Error()})
				} else if expired > 0 {
					m.publishDesktopApprovalRequestsChange()
				}
			}
			m.addFact(taskID, poc.Fact{Message: "approve timed out waiting for join_request"})
			m.addSuggestion(taskID, poc.Suggestion{Message: "verify joiner is running: miopunch join <invite_code>"})
			m.done(taskID, poc.ReasonCodeTimeout, poc.ExitCodeTimeout)
			return
		case ev := <-evCh:
			if ev.Err != nil {
				m.addFact(taskID, poc.Fact{Message: "mqtt error: " + ev.Err.Error()})
				continue
			}
			if ev.Topic != code.InviteTopic {
				continue
			}

			joinReq, err := validateInviteJoinRequest(inviteSecret, code.InviteTopic, selfID.PeerID, ev.Payload)
			if err != nil {
				continue
			}

			if args.ExplicitReview {
				if err := m.recordExplicitApprovalRequest(taskID, store, code, inviteID, joinReq); err != nil {
					m.addFact(taskID, poc.Fact{Message: "record approval request failed: " + err.Error()})
				}
				continue
			}

			ct, hit, err := idem.Handle(joinReq.req, code.InviteTopic, code.ExpiresAtUnixMs, code.MaxUses, func() ([]byte, error) {
				return m.buildMembershipBundleCiphertext(stateDir, selfID, netState, head, code, joinReq)
			})
			if err != nil {
				if errors.Is(err, controlplane.ErrInviteExpired) {
					m.addFact(taskID, poc.Fact{Message: "invite expired"})
					m.done(taskID, poc.ReasonCodeTimeout, poc.ExitCodeTimeout)
					return
				}
				if errors.Is(err, controlplane.ErrInviteUsesExhausted) {
					m.addFact(taskID, poc.Fact{Message: "invite uses exhausted"})
					m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
					return
				}
				m.addFact(taskID, poc.Fact{Message: "handle join_request failed: " + err.Error()})
				continue
			}

			pubCtx, cancelPub := context.WithTimeout(ctx, 10*time.Second)
			pubErr := publishMQTTAny(pubCtx, mbs, joinReq.replyTopic, ct)
			cancelPub()
			if pubErr != nil {
				m.addFact(taskID, poc.Fact{Message: "publish membership_bundle failed: " + pubErr.Error()})
				continue
			}

			m.addFact(taskID, poc.Fact{TermID: "member_peer_id", Message: "member_peer_id=" + joinReq.memberPeerID})
			m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("idempotency_hit=%v", hit)})
			m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("known_seed_peers=%d", len(st.Peers)+1)})
			if !hit {
				approvedUnique++
			}
			if approvedUnique >= code.MaxUses {
				m.addSuggestion(taskID, poc.Suggestion{Message: "approved max uses; invite complete"})
				m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
				return
			}

			// Default POC: stop after the first successful approval.
			if code.MaxUses == 1 && !hit {
				m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
				return
			}
		}
	}
}

func validateInviteJoinRequest(inviteSecret []byte, inviteTopic string, selfPeerID string, payload []byte) (validatedJoinRequest, error) {
	joinReqJSON, err := controlplane.OpenInviteJoinRequestV0(inviteSecret, inviteTopic, payload)
	if err != nil {
		return validatedJoinRequest{}, err
	}

	req, err := controlplane.UnmarshalMessage(joinReqJSON)
	if err != nil {
		return validatedJoinRequest{}, err
	}
	if strings.TrimSpace(req.Signed.Kind) != joinRequestKindV0 {
		return validatedJoinRequest{}, errors.New("unexpected join request kind")
	}

	var body joinRequestBodyV0
	if err := json.Unmarshal(req.Signed.Body, &body); err != nil {
		return validatedJoinRequest{}, err
	}
	replyTopic := strings.TrimSpace(body.ReplyTopic)
	if replyTopic == "" {
		return validatedJoinRequest{}, errors.New("empty reply topic")
	}

	memberEdPubBytes, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(body.Ed25519PubB64))
	if err != nil || len(memberEdPubBytes) != ed25519.PublicKeySize {
		return validatedJoinRequest{}, errors.New("invalid member ed25519 public key")
	}
	memberEdPub := ed25519.PublicKey(memberEdPubBytes)
	memberPeerID, err := controlplane.PeerIDFromEd25519Pub(memberEdPub)
	if err != nil {
		return validatedJoinRequest{}, err
	}
	if req.Signed.SenderPeerID != memberPeerID {
		return validatedJoinRequest{}, errors.New("sender peer mismatch")
	}

	memberXPub, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(body.X25519PubB64))
	if err != nil || len(memberXPub) != 32 {
		return validatedJoinRequest{}, errors.New("invalid member x25519 public key")
	}

	if err := controlplane.VerifyV0ForSelf(selfPeerID, memberEdPub, req); err != nil {
		return validatedJoinRequest{}, err
	}

	return validatedJoinRequest{
		req:          req,
		body:         body,
		replyTopic:   replyTopic,
		memberPeerID: memberPeerID,
		memberXPub:   memberXPub,
	}, nil
}

func (m *Manager) recordExplicitApprovalRequest(taskID string, store *controlplane.InviteStore, code controlplane.InviteCodeV0, inviteID string, joinReq validatedJoinRequest) error {
	rt, ok := m.approveRuntime(taskID)
	if !ok {
		return errors.New("approval runtime is not active")
	}

	nowUnixMs := time.Now().UTC().UnixMilli()
	if err := controlplane.ValidateRPCRequestTime(nowUnixMs, joinReq.req); err != nil {
		return err
	}
	if code.ExpiresAtUnixMs > 0 && nowUnixMs > code.ExpiresAtUnixMs {
		return controlplane.ErrInviteExpired
	}
	rec, created, err := store.RecordApprovalRequest(code.InviteTopic, code.ExpiresAtUnixMs, code.MaxUses, controlplane.ApprovalRequestRecord{
		ApproveTaskID:   taskID,
		InviteID:        inviteID,
		RequestMsgID:    joinReq.req.Route.MsgID,
		MemberPeerID:    joinReq.memberPeerID,
		CreatedAtUnixMs: nowUnixMs,
		UpdatedAtUnixMs: nowUnixMs,
		ExpiresAtUnixMs: code.ExpiresAtUnixMs,
		MemberName:      joinReq.body.MemberName,
		PlatformHint:    joinReq.body.PlatformHint,
		V4Hint:          joinReq.body.SeedPeer.v4Hint(),
		V6Hint:          joinReq.body.SeedPeer.v6Hint(),
		DecisionMaterial: &controlplane.ApprovalDecisionMaterial{
			InviteBrokers:                   append([]string(nil), code.InviteBrokers...),
			ReplyTopic:                      joinReq.replyTopic,
			JoinRequestBodyB64URL:           base64.RawURLEncoding.EncodeToString(joinReq.req.Signed.Body),
			MemberEd25519PubB64:             strings.TrimSpace(joinReq.body.Ed25519PubB64),
			MemberX25519PubB64:              strings.TrimSpace(joinReq.body.X25519PubB64),
			ValidatedAtUnixMs:               nowUnixMs,
			ValidatedRequestExpiresAtUnixMs: joinReq.req.Route.ExpiresAtUnixMs,
			ValidatedRequestSenderID:        joinReq.req.Signed.SenderPeerID,
		},
	})
	if err != nil {
		return err
	}

	rt.upsertRequest(approveRuntimeRequest{
		req:          joinReq.req,
		body:         joinReq.body,
		replyTopic:   joinReq.replyTopic,
		memberPeerID: joinReq.memberPeerID,
		memberXPub:   append([]byte(nil), joinReq.memberXPub...),
	})
	if created {
		m.addFact(taskID, poc.Fact{TermID: "approval_request", Message: "approval_request=" + rec.RequestMsgID})
		m.addFact(taskID, poc.Fact{TermID: "member_peer_id", Message: "member_peer_id=" + rec.MemberPeerID})
	} else {
		m.addFact(taskID, poc.Fact{Message: "approval_request_duplicate=" + rec.RequestMsgID})
	}
	m.publishDesktopApprovalRequestsChange()
	return nil
}

func (m *Manager) buildMembershipBundleCiphertext(stateDir string, selfID pocstate.Identity, netState pocstate.Net, head pocstate.GovernanceHeadSnapshotV1, code controlplane.InviteCodeV0, joinReq validatedJoinRequest) ([]byte, error) {
	st, err := m.loadState()
	if err != nil {
		return nil, err
	}
	if st.Local == nil {
		st.Local = &pocstate.LocalConfig{}
	}
	st.Local.NormalizeDefaults()

	body := joinReq.body
	if body.SeedPeer != nil {
		body.SeedPeer.PeerID = joinReq.memberPeerID
		body.SeedPeer.SetMQTTBrokers(netState.BrokersEffective)
		if cfg, ok := body.SeedPeer.peerConfig(); ok {
			st.UpsertPeer(joinReq.memberPeerID, cfg)
			if err := m.saveState(st); err != nil {
				return nil, err
			}
		}
	}

	decl, err := pocstate.NewApproveMemberDeclV0(time.Now().UTC(), selfID, pocstate.ApproveMemberBodyV0{
		MemberPeerID:  joinReq.memberPeerID,
		MemberName:    body.MemberName,
		Ed25519PubB64: strings.TrimSpace(body.Ed25519PubB64),
		X25519PubB64:  strings.TrimSpace(body.X25519PubB64),
		V4Hint:        body.SeedPeer.v4Hint(),
		V6Hint:        body.SeedPeer.v6Hint(),
		PlatformHint:  body.PlatformHint,
	})
	if err != nil {
		return nil, err
	}

	declsFile, err := pocstate.UpdateDecls(stateDir, func(f *pocstate.DeclsFileV0) error {
		f.Decls = pocstate.AddDeclSetUnionV0(f.Decls, decl)
		return nil
	})
	if err != nil {
		return nil, err
	}

	recommendations := bootstrapRecommendations(selfID.PeerID, joinReq.memberPeerID, st)
	seedPeers := seedPeersForRecommendations(recommendations, selfID.PeerID, st)
	bundle := membershipBundleV0{
		NetID:                    netState.NetID,
		NetSecretB64:             base64.RawURLEncoding.EncodeToString(netState.NetSecret),
		BrokersEffective:         netState.BrokersEffective,
		GovernanceHeadSnapshot:   head,
		Decls:                    declsFile.Decls,
		SeedPeers:                seedPeers,
		BootstrapRecommendations: recommendations,
	}
	pt, err := json.Marshal(bundle)
	if err != nil {
		return nil, fmt.Errorf("marshal membership_bundle: %w", err)
	}
	return controlplane.SealInviteMembershipBundleV0(selfID.X25519Priv, joinReq.memberXPub, code.InviteTopic, selfID.PeerID, joinReq.memberPeerID, pt)
}
