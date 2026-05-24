package task

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

const (
	joinRequestKindV0 = "join_request"
)

type joinRequestBodyV0 struct {
	ReplyTopic string `json:"reply_topic"`

	MemberName   string `json:"member_name,omitempty"`
	PlatformHint string `json:"platform,omitempty"`

	Ed25519PubB64 string `json:"ed25519_pub_b64"`
	X25519PubB64  string `json:"x25519_pub_b64"`

	SeedPeer *seedPeerV0 `json:"seed_peer,omitempty"`
}

type membershipBundleV0 struct {
	NetID            string   `json:"net_id"`
	NetSecretB64     string   `json:"net_secret_b64"`
	BrokersEffective []string `json:"brokers_effective,omitempty"`

	GovernanceHeadSnapshot pocstate.GovernanceHeadSnapshotV1 `json:"governance_head_snapshot"`
	Decls                  []pocstate.DeclV0                 `json:"decls"`

	SeedPeers []seedPeerV0 `json:"seed_peers,omitempty"`

	BootstrapRecommendations []pocstate.BootstrapPeerEvidenceV0 `json:"bootstrap_recommendations,omitempty"`
}

type seedPeerV0 struct {
	PeerID string `json:"peer_id"`

	ProxyName   string   `json:"proxy_name"`
	SecretKey   string   `json:"secret_key"`
	MQTTBroker  string   `json:"mqtt_broker"`
	MQTTBrokers []string `json:"mqtt_brokers,omitempty"`
	TopicPrefix string   `json:"topic_prefix"`

	V4Hint string `json:"v4_hint,omitempty"`
	V6Hint string `json:"v6_hint,omitempty"`

	DataProto string `json:"data_proto"`
	QUICCC    string `json:"quic_cc"`
}

func (m *Manager) runJoinTask(taskID string, rawArgs []byte) {
	var args JoinArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args = args.normalize()
	if args.Code == "" {
		m.addFact(taskID, poc.Fact{Message: "missing invite code"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "use: miopunch join <invite_code-or-url>"})
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
	localSeed, err := m.ensureLocalSeedPeer(selfID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "prepare local seed peer: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	inviteSecret, err := decodeInviteSecretB64(code.InviteSecretB64)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid invite_secret_b64"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	issuerXPub, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(code.IssuerX25519PubB64))
	if err != nil || len(issuerXPub) != 32 {
		m.addFact(taskID, poc.Fact{Message: "invalid issuer_x25519_pub_b64"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	// Join request lifetime is bounded by the invite expiry.
	expiresAt := time.UnixMilli(code.ExpiresAtUnixMs).UTC()
	ctx, cancel := context.WithDeadline(m.ctx, expiresAt)
	defer cancel()

	replyTopic, err := newRandomTopic()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "new reply_topic: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	now := time.Now().UTC()
	body := joinRequestBodyV0{
		ReplyTopic:    replyTopic,
		Ed25519PubB64: selfID.Ed25519PubB64(),
		X25519PubB64:  selfID.X25519PubB64(),
		SeedPeer:      &localSeed,
	}
	bodyJSON, err := json.Marshal(body)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "marshal join_request body: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	msgID, err := controlplane.NewMsgID()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "new msg_id: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	msgID, err = controlplane.CanonicalizeMsgID(msgID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "canonicalize msg_id: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	joinReq := controlplane.Message{
		ProtoVersion: controlplane.ProtoVersionV0,
		Route: controlplane.Route{
			DstPeerID:       code.IssuerPeerID,
			MsgID:           msgID,
			HopLimit:        0,
			CreatedAtUnixMs: now.UnixMilli(),
			ExpiresAtUnixMs: code.ExpiresAtUnixMs,
		},
		Signed: controlplane.Signed{
			SenderPeerID: selfID.PeerID,
			Kind:         joinRequestKindV0,
			Body:         bodyJSON,
		},
	}
	if err := controlplane.SignV0(selfID.Ed25519Priv, &joinReq); err != nil {
		m.addFact(taskID, poc.Fact{Message: "sign join_request: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	joinReqJSON, err := controlplane.MarshalMessage(joinReq)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "marshal join_request: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	joinReqCT, err := controlplane.SealInviteJoinRequestV0(inviteSecret, code.InviteTopic, joinReqJSON)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "encrypt join_request: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	m.setStage(taskID, poc.StagePeerContact, "connect invite brokers")
	m.addFact(taskID, poc.Fact{TermID: "invite_brokers", Message: "invite_brokers=" + strings.Join(code.InviteBrokers, ",")})

	mbs, brokerFailures, err := openMQTTMailboxes(ctx, code.InviteBrokers, "miopunch-invite-join")
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
	mbs, brokerFailures, err = subscribeMQTTMailboxes(subCtx, mbs, replyTopic)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "subscribe reply_topic failed: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	for _, failure := range brokerFailures {
		m.addFact(taskID, poc.Fact{Message: "mqtt broker skipped: " + failure})
	}
	defer closeMQTTMailboxes(mbs)

	pubOnce := func() error {
		pubCtx, cancelPub := context.WithTimeout(ctx, 10*time.Second)
		defer cancelPub()
		return publishMQTTAny(pubCtx, mbs, code.InviteTopic, joinReqCT)
	}

	if err := pubOnce(); err != nil {
		m.addFact(taskID, poc.Fact{Message: "publish join_request failed: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	m.setStage(taskID, poc.StagePeerContact, "wait membership bundle")

	evCh, stop := fanInMailboxEvents(ctx, mbs)
	defer stop()

	backoff := 500 * time.Millisecond
	nextRetry := time.NewTimer(backoff)
	defer nextRetry.Stop()

	var bundle membershipBundleV0
	gotBundle := false

	for !gotBundle {
		select {
		case <-ctx.Done():
			m.addFact(taskID, poc.Fact{Message: "join_request timed out waiting for membership_bundle"})
			m.addSuggestion(taskID, poc.Suggestion{Message: "verify approver is running: miopunch approve <code>"})
			m.done(taskID, poc.ReasonCodeTimeout, poc.ExitCodeTimeout)
			return
		case <-nextRetry.C:
			if err := pubOnce(); err != nil {
				m.addFact(taskID, poc.Fact{Message: "retry publish join_request failed: " + err.Error()})
			}
			if backoff < 10*time.Second {
				backoff *= 2
				if backoff > 10*time.Second {
					backoff = 10 * time.Second
				}
			}
			nextRetry.Reset(backoff)
		case ev := <-evCh:
			if ev.Err != nil {
				m.addFact(taskID, poc.Fact{Message: "mqtt error: " + ev.Err.Error()})
				continue
			}
			if ev.Topic != replyTopic {
				continue
			}

			pt, err := controlplane.OpenInviteMembershipBundleV0(selfID.X25519Priv, issuerXPub, code.InviteTopic, code.IssuerPeerID, selfID.PeerID, ev.Payload)
			if err != nil {
				continue
			}
			var rejection approvalRejectionV0
			if err := json.Unmarshal(pt, &rejection); err == nil && strings.TrimSpace(rejection.Status) == controlplane.ApprovalStatusRejected {
				m.addFact(taskID, poc.Fact{Message: "join_request rejected"})
				if strings.TrimSpace(rejection.Reason) != "" {
					m.addFact(taskID, poc.Fact{Message: "reason=" + strings.TrimSpace(rejection.Reason)})
				}
				m.done(taskID, poc.ReasonCodeForbidden, poc.ExitCodeForbidden)
				return
			}
			if err := json.Unmarshal(pt, &bundle); err != nil {
				continue
			}
			gotBundle = true
		}
	}

	secret, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(bundle.NetSecretB64))
	if err != nil || len(secret) != 32 {
		m.addFact(taskID, poc.Fact{Message: "invalid membership_bundle net_secret"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	localNetID, err := pocstate.NetIDFromSecret(secret)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "derive net_id: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	if strings.TrimSpace(bundle.NetID) == "" {
		m.addFact(taskID, poc.Fact{Message: "missing membership_bundle net_id"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	if strings.TrimSpace(bundle.NetID) != localNetID {
		m.addFact(taskID, poc.Fact{Message: "membership_bundle net_id mismatch"})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	if err := pocstate.ValidateGovernanceHeadSnapshotBootstrap(localNetID, bundle.GovernanceHeadSnapshot); err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid governance head snapshot: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	if _, err := pocstate.DeclsHeadB64V0(bundle.Decls); err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid decls: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	// Persist long-term state (net/head/decls).
	if err := pocstate.SaveNet(stateDir, pocstate.Net{NetSecret: secret, BrokersEffective: bundle.BrokersEffective}); err != nil {
		m.addFact(taskID, poc.Fact{Message: "save net: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	if err := pocstate.SaveGovernanceHeadSnapshot(stateDir, bundle.GovernanceHeadSnapshot); err != nil {
		m.addFact(taskID, poc.Fact{Message: "save head snapshot: " + err.Error()})
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
	if _, err := pocstate.UpdateDecls(stateDir, func(f *pocstate.DeclsFileV0) error {
		f.Decls = append([]pocstate.DeclV0(nil), bundle.Decls...)
		return nil
	}); err != nil && !errors.Is(err, os.ErrNotExist) {
		m.addFact(taskID, poc.Fact{Message: "save decls: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	recommendations := append([]pocstate.BootstrapPeerEvidenceV0(nil), bundle.BootstrapRecommendations...)
	if len(recommendations) == 0 {
		for _, sp := range bundle.SeedPeers {
			recommendations = append(recommendations, pocstate.BootstrapPeerEvidenceV0{
				PeerID: strings.TrimSpace(sp.PeerID),
				Bucket: "unknown",
				Reason: "membership_seed",
			})
		}
	}
	if err := pocstate.SaveBootstrap(stateDir, pocstate.BootstrapFileV0{
		Recommendations: recommendations,
	}); err != nil {
		m.addFact(taskID, poc.Fact{Message: "save bootstrap evidence: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	// Bridge seeds into state.json for existing punching/dialer.
	st, err := m.loadState()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "load state: " + err.Error()})
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
			m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
			return
		}
		st.Local.SecretKey = secretKey
	}
	st.EnsureLocalDefaults()
	st.Local.SetMQTTBrokers(bundle.BrokersEffective)

	seedCount := 0
	for _, sp := range bundle.SeedPeers {
		if strings.TrimSpace(sp.PeerID) == "" {
			continue
		}
		cfg, ok := sp.peerConfig()
		if !ok {
			continue
		}
		st.UpsertPeer(sp.PeerID, cfg)
		seedCount++
	}

	if err := m.saveState(st); err != nil {
		m.addFact(taskID, poc.Fact{Message: "save state: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	m.addFact(taskID, poc.Fact{TermID: "peer_id", Message: "peer_id=" + selfID.PeerID})
	m.addFact(taskID, poc.Fact{TermID: "net_id", Message: "net_id=" + bundle.NetID})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("seed_peers=%d", seedCount)})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("bootstrap_recommendations=%d", len(recommendations))})
	m.addSuggestion(taskID, poc.Suggestion{Message: "list peers via: miopunch ls"})
	m.addSuggestion(taskID, poc.Suggestion{Message: "try: miopunch ping <peer_id>"})
	m.addSuggestion(taskID, poc.Suggestion{Message: "try: miopunch sh <peer_id>"})
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}
