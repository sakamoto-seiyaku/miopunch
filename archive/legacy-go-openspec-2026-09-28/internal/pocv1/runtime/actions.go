package runtime

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"sort"
	"strings"
	"time"

	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocv1/enroll"
	"github.com/miopunch/miopunch/internal/pocv1/peere2e"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/presence"
	"github.com/miopunch/miopunch/internal/pocv1/punch"
	"github.com/miopunch/miopunch/internal/pocv1/session"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
	"github.com/miopunch/miopunch/internal/shellproto"
	"github.com/miopunch/miopunch/internal/shelltarget"
	signalmqtt "github.com/miopunch/miopunch/internal/signaling/mqtt"
)

func (r *Runtime) Action(ctx context.Context, action string, raw json.RawMessage) (ActionResult, *problem) {
	if ctx == nil {
		ctx = context.Background()
	}
	action = strings.TrimSpace(action)
	if action == "" {
		problem := newProblem(
			StageNetwork,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"missing action",
			nil,
			[]poc.Suggestion{{Message: "retry with a valid action"}},
		)
		r.setStatus(problem)
		return r.failureResult(problem), problem
	}

	var (
		result ActionResult
		prob   *problem
	)
	switch action {
	case "ls":
		result, prob = r.doLS(ctx)
	case "init-network":
		var args InitNetworkArgs
		if err := parseArgs(raw, &args); err != nil {
			prob = wrapProblem(StageNetwork, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invalid init-network args", err, "retry with valid flags")
			break
		}
		result, prob = r.doInitNetwork(ctx, args)
	case "invite":
		var args InviteArgs
		if err := parseArgs(raw, &args); err != nil {
			prob = wrapProblem(StageNetwork, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invalid invite args", err, "retry with valid flags")
			break
		}
		result, prob = r.doInvite(ctx, args)
	case "approve":
		var args ApproveArgs
		if err := parseArgs(raw, &args); err != nil {
			prob = wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invalid approve args", err, "retry with a valid invite code")
			break
		}
		result, prob = r.doApprove(ctx, args)
	case "join":
		var args JoinArgs
		if err := parseArgs(raw, &args); err != nil {
			prob = wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invalid join args", err, "retry with a valid invite code")
			break
		}
		result, prob = r.doJoin(ctx, args)
	case "ping":
		var args PingArgs
		if err := parseArgs(raw, &args); err != nil {
			prob = wrapProblem(StageSecureSession, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invalid ping args", err, "retry with a valid peer_id")
			break
		}
		result, prob = r.doPing(ctx, args)
	case "sh_ls":
		var args ShellArgs
		if err := parseArgs(raw, &args); err != nil {
			prob = wrapProblem(StageShell, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invalid sh ls args", err, "retry with a valid peer_id")
			break
		}
		result, prob = r.doShellList(ctx, args)
	case "sh":
		var args ShellArgs
		if err := parseArgs(raw, &args); err != nil {
			prob = wrapProblem(StageShell, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invalid sh args", err, "retry with a valid peer_id")
			break
		}
		result, prob = r.doShell(ctx, args)
	case "revoke":
		var args RevokeArgs
		if err := parseArgs(raw, &args); err != nil {
			prob = wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invalid revoke args", err, "retry with a valid peer_id")
			break
		}
		result, prob = r.doRevoke(ctx, args)
	default:
		prob = newProblem(
			StageNetwork,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"unsupported action",
			[]poc.Fact{{Message: "action=" + action}},
			[]poc.Suggestion{{Message: "retry with a supported action"}},
		)
	}

	if prob != nil {
		r.setStatus(prob)
		return r.failureResult(prob), prob
	}
	r.clearStatus()
	return result, nil
}

func appendDiagnosticFacts(base []poc.Fact, values ...string) []poc.Fact {
	out := append([]poc.Fact(nil), base...)
	for _, value := range values {
		value = strings.TrimSpace(value)
		if value == "" {
			continue
		}
		out = append(out, poc.Fact{Message: value})
	}
	return out
}

func appendProblemFacts(prob *problem, values ...string) *problem {
	if prob == nil {
		return nil
	}
	prob.facts = appendDiagnosticFacts(prob.facts, values...)
	return prob
}

func selectedPathFromSession(sess session.PeerSession) string {
	return strings.TrimSpace(dataplane.PathFactsFromSession(sess).SelectedPath)
}

func appendSessionPathProblemFacts(prob *problem, sess session.PeerSession) *problem {
	if prob == nil {
		return nil
	}
	selectedPath := selectedPathFromSession(sess)
	if selectedPath == "" {
		return prob
	}
	return appendProblemFacts(prob, "selected_path="+selectedPath)
}

func appendPathResultProblemFacts(prob *problem, result punch.PathResult) *problem {
	if prob == nil {
		return nil
	}
	values := []string{}
	if selectedPath := strings.TrimSpace(result.Evidence.SelectedPath); selectedPath != "" {
		values = append(values, "selected_path="+selectedPath)
	}
	if ownership := strings.TrimSpace(string(result.Ownership())); ownership != "" {
		values = append(values, "selected_udp_ownership="+ownership)
	}
	if value := candidateFact("selected_local_candidate", result.Evidence.SelectedLocal); value != "" {
		values = append(values, value)
	}
	if value := candidateFact("selected_remote_candidate", result.Evidence.SelectedRemote); value != "" {
		values = append(values, value)
	}
	if remoteUDP := strings.TrimSpace(result.Evidence.SelectedRemoteUDP); remoteUDP != "" {
		values = append(values, "selected_remote_udp="+remoteUDP)
	} else if result.RemoteAddr != nil {
		values = append(values, "selected_remote_udp="+result.RemoteAddr.String())
	}
	return appendProblemFacts(prob, values...)
}

func candidateFact(name string, candidate punch.Candidate) string {
	name = strings.TrimSpace(name)
	addr := strings.TrimSpace(candidate.Addr)
	if name == "" || addr == "" {
		return ""
	}
	kind := strings.TrimSpace(string(candidate.Kind))
	if kind == "" {
		return name + "=" + addr
	}
	return name + "=" + kind + "@" + addr
}

func (r *Runtime) doLS(ctx context.Context) (ActionResult, *problem) {
	if err := r.ensureWorkers(ctx); err != nil {
		return ActionResult{}, wrapProblem(StageDiscover, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to start discover runtime", err, "retry")
	}
	r.mu.Lock()
	networkID := strings.TrimSpace(r.meta.ActiveNetworkID)
	r.mu.Unlock()
	if networkID == "" {
		return ActionResult{}, newProblem(
			StageNetwork,
			poc.ReasonCodeNotFound,
			poc.ExitCodeNotFound,
			"no active network is joined",
			nil,
			[]poc.Suggestion{
				{Message: "run: miopunch init-network"},
				{Message: "or: miopunch join <invite_code>"},
			},
		)
	}

	discover := r.Snapshot().DiscoverView
	entries := make([]presence.DiscoverProjectionPeer, 0, len(discover.Peers))
	entries = append(entries, discover.Peers...)
	sort.Slice(entries, func(i, j int) bool {
		return entries[i].PeerID < entries[j].PeerID
	})

	lines := make([]string, 0, len(entries))
	facts := []poc.Fact{{Message: fmt.Sprintf("peer_count=%d", len(entries))}}
	for _, entry := range entries {
		lines = append(lines, entry.PeerID)
		facts = append(facts, poc.Fact{
			Message: fmt.Sprintf("peer_id=%s online_state=%s", entry.PeerID, entry.OnlineState),
		})
	}

	data := mustJSONMarshal(map[string]any{
		"peers": entries,
	})
	return r.successResult(lines, facts, nil, data, ""), nil
}

func (r *Runtime) doInitNetwork(ctx context.Context, args InitNetworkArgs) (ActionResult, *problem) {
	if args.CreateNew && strings.TrimSpace(args.Confirm) != "create-new-network" {
		return ActionResult{}, newProblem(
			StageNetwork,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"creating a new network requires --confirm create-new-network",
			nil,
			[]poc.Suggestion{{Message: "retry with: miopunch init-network --new --confirm create-new-network"}},
		)
	}

	r.mu.Lock()
	activeNetworkID := strings.TrimSpace(r.meta.ActiveNetworkID)
	r.mu.Unlock()
	if activeNetworkID != "" {
		return ActionResult{}, newProblem(
			StageNetwork,
			poc.ReasonCodeConflict,
			poc.ExitCodeConflict,
			"an active network is already joined",
			[]poc.Fact{{Message: "network_id=" + activeNetworkID}},
			[]poc.Suggestion{{Message: "remove the existing runtime state before creating a new network"}},
		)
	}

	brokerEndpoint := r.currentBrokerEndpoint()
	var (
		broker *embeddedBroker
		err    error
	)
	if strings.TrimSpace(brokerEndpoint) == "" {
		broker, err = startEmbeddedBroker("")
		if err != nil {
			return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to start embedded broker", err, "retry")
		}
		defer func() {
			if broker != nil {
				_ = broker.Close()
			}
		}()
		brokerEndpoint = broker.Endpoint()
	} else {
		brokerEndpoint = normalizeBrokerEndpoint(brokerEndpoint)
	}

	deviceKeys, err := r.store.EnsureDeviceKeys()
	if err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to ensure device keys", err, "retry")
	}
	localPriv, err := deviceKeys.Ed25519PrivateKey()
	if err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to derive local signing key", err, "retry")
	}
	localPub, err := deviceKeys.Ed25519PublicKey()
	if err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to derive local public key", err, "retry")
	}
	localX25519Pub, err := deviceKeys.X25519PublicKey()
	if err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to derive local x25519 public key", err, "retry")
	}
	peerID, err := deviceKeys.PeerID()
	if err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to derive local peer_id", err, "retry")
	}

	rawNetworkID := make([]byte, wire.RawIDLen)
	if _, err := rand.Read(rawNetworkID); err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to generate network id", err, "retry")
	}
	networkID, err := wire.EncodeNetworkID(rawNetworkID)
	if err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to encode network id", err, "retry")
	}
	mailboxSecret := make([]byte, 32)
	if _, err := rand.Read(mailboxSecret); err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to generate mailbox secret", err, "retry")
	}

	credential := enroll.MemberCredential{
		NetworkID:         networkID,
		SubjectEd25519Pub: append([]byte(nil), localPub...),
		SubjectX25519Pub:  append([]byte(nil), localX25519Pub...),
		Role:              "admin",
		NotBeforeUnixMs:   uint64(time.Now().UTC().UnixMilli()),
		NotAfterUnixMs:    uint64(time.Now().Add(365 * 24 * time.Hour).UTC().UnixMilli()),
		IssuerKeyID:       "self",
	}
	if err := enroll.SignMemberCredential(localPriv, &credential); err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to sign self member credential", err, "retry")
	}
	credentialBytes, err := credential.MarshalBinary()
	if err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to encode self member credential", err, "retry")
	}

	joined := persist.JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: credentialBytes,
		MailboxSecret:        append([]byte(nil), mailboxSecret...),
		RuntimeBroker:        persist.RuntimeBroker{Endpoint: brokerEndpoint},
		RosterSnapshot: persist.RosterSnapshot{
			Entries: []persist.RosterEntry{
				{
					PeerID:           peerID,
					MemberCredential: credentialBytes,
					DeviceName:       r.deviceName,
					Platform:         r.platform,
				},
			},
		},
	}
	if err := r.store.PersistJoinedBootstrap(joined); err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to persist joined bootstrap", err, "retry")
	}

	r.mu.Lock()
	brokerOverride := strings.TrimSpace(r.meta.RuntimeBrokerOverride)
	r.mu.Unlock()
	meta := metadata{
		ActiveNetworkID:        networkID,
		AuthorityEd25519PubB64: encodeKeyB64(localPub),
		AuthorityX25519PubB64:  encodeKeyB64(localX25519Pub),
		Role:                   "admin",
		BrokerEndpoint:         brokerEndpoint,
		RuntimeBrokerOverride:  brokerOverride,
	}
	if err := saveMetadata(r.root, meta); err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to persist runtime metadata", err, "retry")
	}
	r.mu.Lock()
	r.meta = meta
	r.broker = broker
	broker = nil
	r.mu.Unlock()

	if err := r.ensureWorkers(ctx); err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to start runtime workers", err, "retry")
	}
	r.notifyChange("snapshot.updated")

	facts := []poc.Fact{
		{Message: "network_id=" + networkID},
		{Message: "peer_id=" + peerID},
		{Message: "role=admin"},
		{Message: "broker_endpoint=" + meta.BrokerEndpoint},
	}
	lines := []string{
		"network_id=" + networkID,
		"peer_id=" + peerID,
		"role=admin",
	}
	data := mustJSONMarshal(map[string]any{
		"network_id":      networkID,
		"peer_id":         peerID,
		"role":            "admin",
		"broker_endpoint": meta.BrokerEndpoint,
	})
	return r.successResult(lines, facts, nil, data, ""), nil
}

func (r *Runtime) doInvite(ctx context.Context, args InviteArgs) (ActionResult, *problem) {
	deviceKeys, networkID, localPriv, authorityPub, authorityX25519Pub, metaProblem := r.requireAdminAuthority()
	if metaProblem != nil {
		return ActionResult{}, metaProblem
	}

	if err := r.ensureWorkers(ctx); err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to start runtime workers", err, "retry")
	}

	rawNetworkID, err := wire.DecodeNetworkID(networkID)
	if err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to decode network id", err, "retry")
	}
	topicScope, err := r.store.LoadTopicScope(networkID)
	if err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to load topic scope", err, "retry")
	}
	inviteID, err := wire.NewMsgID()
	if err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to generate invite id", err, "retry")
	}
	expiresAt := time.Now().Add(defaultInviteLifetime)
	if strings.TrimSpace(args.Expires) != "" {
		duration, err := time.ParseDuration(strings.TrimSpace(args.Expires))
		if err != nil {
			return ActionResult{}, newProblem(
				StageNetwork,
				poc.ReasonCodeBadRequest,
				poc.ExitCodeBadRequest,
				"invalid invite expiration",
				[]poc.Fact{{Message: "expires=" + strings.TrimSpace(args.Expires)}},
				[]poc.Suggestion{{Message: "use a Go duration such as 15m or 1h"}},
			)
		}
		expiresAt = time.Now().Add(duration)
	}
	invite := enroll.InviteCapability{
		NetworkIDBytes:      rawNetworkID,
		AuthorityEd25519Pub: append([]byte(nil), authorityPub...),
		AuthorityX25519Pub:  append([]byte(nil), authorityX25519Pub...),
		BrokerEndpoint:      r.currentBrokerEndpoint(),
		JoinTopic:           joinTopic(topicScope),
		InviteID:            inviteID,
		NotAfterUnixMs:      uint64(expiresAt.UTC().UnixMilli()),
	}
	if err := enroll.SignInviteCapability(localPriv, &invite); err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to sign invite capability", err, "retry")
	}
	code, err := invite.InviteCode()
	if err != nil {
		return ActionResult{}, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to encode invite code", err, "retry")
	}

	peerID, _ := deviceKeys.PeerID()
	facts := []poc.Fact{
		{Message: "invite_code=" + code},
		{Message: "invite_id=" + invite.InviteID},
		{Message: "network_id=" + networkID},
		{Message: "peer_id=" + peerID},
		{Message: "join_topic=" + invite.JoinTopic},
		{Message: "broker_endpoint=" + invite.BrokerEndpoint},
	}
	lines := []string{"invite_code=" + code}
	data := mustJSONMarshal(map[string]any{
		"invite_code":     code,
		"network_id":      networkID,
		"join_topic":      invite.JoinTopic,
		"broker_endpoint": invite.BrokerEndpoint,
	})
	report := markdownReport("invite", facts, nil)
	return r.successResult(lines, facts, nil, data, report), nil
}

func (r *Runtime) doApprove(ctx context.Context, args ApproveArgs) (ActionResult, *problem) {
	_, networkID, localPriv, _, authorityX25519Pub, metaProblem := r.requireAdminAuthority()
	if metaProblem != nil {
		return ActionResult{}, metaProblem
	}
	code := strings.TrimSpace(args.Code)
	if code == "" {
		return ActionResult{}, newProblem(
			StageEnroll,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"missing invite code",
			nil,
			[]poc.Suggestion{{Message: "use: miopunch approve <invite_code>"}},
		)
	}
	invite, err := enroll.ParseInviteCode(code)
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invalid invite code", err, "retry with a valid invite code")
	}
	if err := enroll.VerifyInviteCapability(invite); err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invite signature verification failed", err, "retry with a valid invite code")
	}
	inviteNetworkID, err := invite.NetworkID()
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invalid invite network id", err, "retry with a valid invite code")
	}
	if inviteNetworkID != networkID {
		return ActionResult{}, newProblem(
			StageEnroll,
			poc.ReasonCodeConflict,
			poc.ExitCodeConflict,
			"invite does not belong to the active network",
			[]poc.Fact{
				{Message: "active_network_id=" + networkID},
				{Message: "invite_network_id=" + inviteNetworkID},
			},
			[]poc.Suggestion{{Message: "generate a fresh invite from the active admin daemon"}},
		)
	}
	if err := r.ensureWorkers(ctx); err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to start runtime workers", err, "retry")
	}

	timeoutCtx, timeoutCancel := withDefaultTimeout(ctx, defaultApprovalWait)
	defer timeoutCancel()
	sessionCtx, cancel := context.WithCancel(timeoutCtx)
	defer cancel()

	peerSession, err := signalmqtt.OpenPeerMessageSession(sessionCtx, signalmqtt.PeerMessageConfig{
		BrokerURL:       normalizeBrokerEndpoint(invite.BrokerEndpoint),
		SubscribeTopics: []string{invite.JoinTopic},
	})
	if err != nil {
		return ActionResult{}, appendProblemFacts(
			wrapProblem(StageEnroll, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to open approval signaling session", err, "check broker reachability and retry"),
			"network_id="+networkID,
			"invite_id="+invite.InviteID,
			"join_topic="+invite.JoinTopic,
			"broker_endpoint="+normalizeBrokerEndpoint(invite.BrokerEndpoint),
		)
	}
	defer peerSession.Close()
	logutil.Debugf(
		"enroll approve signaling ready: network_id=%s invite_id=%s broker=%s join_topic=%s",
		networkID,
		invite.InviteID,
		normalizeBrokerEndpoint(invite.BrokerEndpoint),
		invite.JoinTopic,
	)

	deviceKeys, err := r.store.LoadDeviceKeys()
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to load device keys", err, "retry")
	}
	logutil.Debugf(
		"enroll approve waiting for join request: network_id=%s invite_id=%s join_topic=%s",
		networkID,
		invite.InviteID,
		invite.JoinTopic,
	)
	opened, err := peerSession.WaitOpened(sessionCtx, deviceKeys.X25519PrivateKey, peere2e.OpenOptions{})
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeTimeout, poc.ExitCodeTimeout, "timed out waiting for join request", err, "retry after the joiner starts")
	}
	logutil.Debugf(
		"enroll approve received peer message: network_id=%s invite_id=%s topic=%s msg_id=%s sender_peer_id=%s kind=%s",
		networkID,
		invite.InviteID,
		opened.Topic,
		opened.Outer.MsgID,
		opened.Inner.SenderPeerID,
		opened.Inner.Kind,
	)
	joinRequest, err := enroll.UnmarshalJoinRequest(opened.Inner.Body)
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "failed to decode join request", err, "retry")
	}
	requestPeerID, err := joinRequest.PeerID()
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "join request peer_id is invalid", err, "retry")
	}

	rosterSnapshot, err := r.store.LoadRosterSnapshot(networkID)
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to load roster snapshot", err, "retry")
	}
	mailboxSecret, err := r.store.LoadMailboxSecret(networkID)
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to load mailbox secret", err, "retry")
	}
	runtimeBroker, err := r.store.LoadRuntimeBroker(networkID)
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to load runtime broker", err, "retry")
	}

	memberCredential := enroll.MemberCredential{
		NetworkID:         networkID,
		SubjectEd25519Pub: append([]byte(nil), joinRequest.RequesterEd25519Pub...),
		SubjectX25519Pub:  append([]byte(nil), joinRequest.RequesterX25519Pub...),
		Role:              "member",
		NotBeforeUnixMs:   uint64(time.Now().UTC().UnixMilli()),
		NotAfterUnixMs:    uint64(time.Now().Add(365 * 24 * time.Hour).UTC().UnixMilli()),
		IssuerKeyID:       "authority",
	}
	if err := enroll.SignMemberCredential(localPriv, &memberCredential); err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to sign member credential", err, "retry")
	}

	enrollRoster, err := enrollRosterFromPersist(rosterSnapshot)
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to build roster snapshot", err, "retry")
	}
	enrollRoster.Entries = upsertEnrollRosterEntry(enrollRoster.Entries, enroll.RosterEntry{
		PeerID:           requestPeerID,
		MemberCredential: memberCredential,
		DeviceName:       joinRequest.DeviceName,
		Platform:         joinRequest.Platform,
	})
	response := enroll.EnrollResponse{
		SelfMemberCredential: memberCredential,
		MailboxSecret:        append([]byte(nil), mailboxSecret...),
		RuntimeBroker:        enroll.RuntimeBroker{Endpoint: runtimeBroker.Endpoint},
		RosterSnapshot:       enrollRoster,
	}

	sealed, hit, err := enroll.AuthorityHandleJoinRequest(
		r.store,
		networkID,
		opened.Outer.MsgID,
		wire.OpenedMessage{Outer: opened.Outer, Inner: opened.Inner},
		localPriv,
		authorityX25519Pub,
		response,
	)
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "join request validation failed", err, "retry with a fresh invite")
	}
	if err := peerSession.PublishPayload(sessionCtx, joinRequest.ReplyTopic, sealed); err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to publish enroll response", err, "check broker reachability and retry")
	}
	logutil.Debugf(
		"enroll approve published response: network_id=%s invite_id=%s approved_peer_id=%s reply_topic=%s replay_cache_hit=%t",
		networkID,
		invite.InviteID,
		requestPeerID,
		joinRequest.ReplyTopic,
		hit,
	)

	if !hit {
		persistRoster, err := enrollRoster.ToPersist()
		if err != nil {
			return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to encode updated roster snapshot", err, "retry")
		}
		if err := r.store.ReplaceRosterSnapshot(networkID, persistRoster); err != nil {
			return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to persist updated roster snapshot", err, "retry")
		}
		if err := r.refreshPresenceRoster(ctx); err != nil {
			return ActionResult{}, wrapProblem(StageDiscover, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to refresh presence roster", err, "retry")
		}
	}

	facts := []poc.Fact{
		{Message: "network_id=" + networkID},
		{Message: "invite_id=" + invite.InviteID},
		{Message: "broker_endpoint=" + normalizeBrokerEndpoint(invite.BrokerEndpoint)},
		{Message: "approved_peer_id=" + requestPeerID},
		{Message: "reply_topic=" + joinRequest.ReplyTopic},
		{Message: "replay_cache_hit=" + fmt.Sprintf("%t", hit)},
	}
	lines := []string{"approved_peer_id=" + requestPeerID}
	data := mustJSONMarshal(map[string]any{
		"peer_id":          requestPeerID,
		"reply_topic":      joinRequest.ReplyTopic,
		"replay_cache_hit": hit,
	})
	report := markdownReport("approve", facts, nil)
	r.notifyChange("snapshot.updated")
	return r.successResult(lines, facts, nil, data, report), nil
}

func (r *Runtime) doJoin(ctx context.Context, args JoinArgs) (ActionResult, *problem) {
	code := strings.TrimSpace(args.Code)
	if code == "" {
		return ActionResult{}, newProblem(
			StageEnroll,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"missing invite code",
			nil,
			[]poc.Suggestion{{Message: "use: miopunch join <invite_code>"}},
		)
	}
	invite, err := enroll.ParseInviteCode(code)
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invalid invite code", err, "retry with a valid invite code")
	}
	if err := enroll.VerifyInviteCapability(invite); err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invite signature verification failed", err, "retry with a valid invite code")
	}
	networkID, err := invite.NetworkID()
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invite network id is invalid", err, "retry with a valid invite code")
	}

	r.mu.Lock()
	activeNetworkID := strings.TrimSpace(r.meta.ActiveNetworkID)
	r.mu.Unlock()
	if activeNetworkID != "" && activeNetworkID != networkID {
		return ActionResult{}, newProblem(
			StageEnroll,
			poc.ReasonCodeConflict,
			poc.ExitCodeConflict,
			"runtime is already joined to a different network",
			[]poc.Fact{
				{Message: "active_network_id=" + activeNetworkID},
				{Message: "invite_network_id=" + networkID},
			},
			[]poc.Suggestion{{Message: "use a clean state root before joining a different network"}},
		)
	}

	deviceKeys, err := r.store.EnsureDeviceKeys()
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to ensure device keys", err, "retry")
	}
	localPriv, err := deviceKeys.Ed25519PrivateKey()
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to derive local signing key", err, "retry")
	}
	localPub, err := deviceKeys.Ed25519PublicKey()
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to derive local public key", err, "retry")
	}
	localX25519Pub, err := deviceKeys.X25519PublicKey()
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to derive local x25519 public key", err, "retry")
	}
	localPeerID, err := deviceKeys.PeerID()
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to derive local peer_id", err, "retry")
	}

	replySuffix, err := wire.NewMsgID()
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to generate reply topic id", err, "retry")
	}
	replyTopic := fmt.Sprintf("mp/v1/reply/%s/%s", localPeerID, strings.ToLower(replySuffix))
	joinRequest := enroll.JoinRequest{
		InviteID:            invite.InviteID,
		RequesterEd25519Pub: append([]byte(nil), localPub...),
		RequesterX25519Pub:  append([]byte(nil), localX25519Pub...),
		ReplyTopic:          replyTopic,
		DeviceName:          r.deviceName,
		Platform:            r.platform,
		CreatedAtUnixMs:     uint64(time.Now().UTC().UnixMilli()),
		ExpiresAtUnixMs:     invite.NotAfterUnixMs,
	}
	if err := enroll.SignJoinRequest(localPriv, &joinRequest); err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to sign join request", err, "retry")
	}
	authorityPeerID, err := wire.PeerIDFromEd25519Pub(invite.AuthorityEd25519Pub)
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "invite authority peer_id is invalid", err, "retry with a valid invite code")
	}
	body, err := joinRequest.MarshalBinary()
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to encode join request", err, "retry")
	}
	msgID, err := wire.NewMsgID()
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to generate join request id", err, "retry")
	}
	inner := wire.InnerMessage{
		DstPeerID:       authorityPeerID,
		MsgID:           msgID,
		CreatedAtUnixMs: joinRequest.CreatedAtUnixMs,
		ExpiresAtUnixMs: joinRequest.ExpiresAtUnixMs,
		SenderPeerID:    localPeerID,
		SenderEd25519:   append([]byte(nil), localPub...),
		Kind:            wire.KindJoinRequest,
		Body:            body,
	}
	if err := wire.SignInner(localPriv, &inner); err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to sign join wire message", err, "retry")
	}

	signaling, err := signalmqtt.OpenPeerMessageSession(ctx, signalmqtt.PeerMessageConfig{
		BrokerURL:       normalizeBrokerEndpoint(invite.BrokerEndpoint),
		SubscribeTopics: []string{replyTopic},
	})
	if err != nil {
		return ActionResult{}, appendProblemFacts(
			wrapProblem(StageEnroll, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to open join signaling session", err, "check broker reachability and retry"),
			"network_id="+networkID,
			"invite_id="+invite.InviteID,
			"peer_id="+localPeerID,
			"join_topic="+invite.JoinTopic,
			"broker_endpoint="+normalizeBrokerEndpoint(invite.BrokerEndpoint),
		)
	}
	defer signaling.Close()
	logutil.Debugf(
		"enroll join signaling ready: network_id=%s invite_id=%s peer_id=%s broker=%s join_topic=%s reply_topic=%s",
		networkID,
		invite.InviteID,
		localPeerID,
		normalizeBrokerEndpoint(invite.BrokerEndpoint),
		invite.JoinTopic,
		replyTopic,
	)

	if _, err := signaling.PublishInner(ctx, invite.JoinTopic, inner, invite.AuthorityX25519Pub, peere2e.SealOptions{}); err != nil {
		return ActionResult{}, appendProblemFacts(
			wrapProblem(StageEnroll, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to publish join request", err, "check broker reachability and retry"),
			"network_id="+networkID,
			"invite_id="+invite.InviteID,
			"peer_id="+localPeerID,
			"join_topic="+invite.JoinTopic,
			"reply_topic="+replyTopic,
			"broker_endpoint="+normalizeBrokerEndpoint(invite.BrokerEndpoint),
		)
	}
	logutil.Debugf(
		"enroll join published request: network_id=%s invite_id=%s peer_id=%s join_topic=%s reply_topic=%s msg_id=%s",
		networkID,
		invite.InviteID,
		localPeerID,
		invite.JoinTopic,
		replyTopic,
		msgID,
	)
	logutil.Debugf(
		"enroll join waiting for response: network_id=%s invite_id=%s peer_id=%s reply_topic=%s",
		networkID,
		invite.InviteID,
		localPeerID,
		replyTopic,
	)
	opened, err := signaling.WaitOpened(ctx, deviceKeys.X25519PrivateKey, peere2e.OpenOptions{})
	if err != nil {
		return ActionResult{}, appendProblemFacts(
			wrapProblem(StageEnroll, poc.ReasonCodeTimeout, poc.ExitCodeTimeout, "timed out waiting for enroll response", err, "retry after the admin approves the invite"),
			"network_id="+networkID,
			"invite_id="+invite.InviteID,
			"peer_id="+localPeerID,
			"join_topic="+invite.JoinTopic,
			"reply_topic="+replyTopic,
			"broker_endpoint="+normalizeBrokerEndpoint(invite.BrokerEndpoint),
		)
	}
	logutil.Debugf(
		"enroll join received response: network_id=%s invite_id=%s peer_id=%s topic=%s msg_id=%s sender_peer_id=%s kind=%s",
		networkID,
		invite.InviteID,
		localPeerID,
		opened.Topic,
		opened.Outer.MsgID,
		opened.Inner.SenderPeerID,
		opened.Inner.Kind,
	)

	response, err := enroll.UnmarshalEnrollResponse(opened.Inner.Body)
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "failed to decode enroll response", err, "retry")
	}
	if err := enroll.VerifyMemberCredential(response.SelfMemberCredential, invite.AuthorityEd25519Pub); err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, "failed to verify member credential", err, "retry with a fresh invite")
	}
	if err := enroll.JoinerPersistBootstrap(r.store, response); err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to persist joined bootstrap", err, "retry")
	}

	meta := metadata{
		ActiveNetworkID:        networkID,
		AuthorityEd25519PubB64: encodeKeyB64(invite.AuthorityEd25519Pub),
		AuthorityX25519PubB64:  encodeKeyB64(invite.AuthorityX25519Pub),
		Role:                   "member",
		BrokerEndpoint:         normalizeBrokerEndpoint(invite.BrokerEndpoint),
	}
	if err := saveMetadata(r.root, meta); err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to persist runtime metadata", err, "retry")
	}
	r.mu.Lock()
	r.meta = meta
	r.mu.Unlock()
	if err := r.ensureWorkers(ctx); err != nil {
		return ActionResult{}, wrapProblem(StageDiscover, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to start joined runtime workers", err, "retry")
	}
	if err := r.refreshPresenceRoster(ctx); err != nil {
		return ActionResult{}, wrapProblem(StageDiscover, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to refresh presence roster", err, "retry")
	}
	r.notifyChange("snapshot.updated")

	facts := []poc.Fact{
		{Message: "network_id=" + networkID},
		{Message: "invite_id=" + invite.InviteID},
		{Message: "peer_id=" + localPeerID},
		{Message: "role=member"},
		{Message: "reply_topic=" + replyTopic},
		{Message: "broker_endpoint=" + meta.BrokerEndpoint},
	}
	lines := []string{
		"network_id=" + networkID,
		"peer_id=" + localPeerID,
		"role=member",
	}
	data := mustJSONMarshal(map[string]any{
		"network_id":      networkID,
		"peer_id":         localPeerID,
		"role":            "member",
		"broker_endpoint": meta.BrokerEndpoint,
	})
	return r.successResult(lines, facts, nil, data, ""), nil
}

func (r *Runtime) doPing(ctx context.Context, args PingArgs) (ActionResult, *problem) {
	peerID := strings.TrimSpace(args.PeerID)
	if peerID == "" {
		return ActionResult{}, newProblem(
			StageSecureSession,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"missing peer_id",
			nil,
			[]poc.Suggestion{{Message: "use: miopunch ping <peer_id>"}},
		)
	}
	policy, problem := normalizePeerPathPolicy(args.P2PNetwork, args.P2PIPFamily)
	if problem != nil {
		return ActionResult{}, problem
	}
	sess, problem := r.ensurePeerSession(ctx, peerID, policy)
	if problem != nil {
		return ActionResult{}, problem
	}
	if problem := r.exchangePing(ctx, sess, peerID); problem != nil {
		r.closePeerSessionOnTransportProblem(sess, problem)
		return ActionResult{}, appendSessionPathProblemFacts(problem, sess)
	}
	selectedPath := selectedPathFromSession(sess)
	facts := []poc.Fact{
		{Message: "peer_id=" + peerID},
		{Message: "ping=ok"},
	}
	lines := []string{"peer_id=" + peerID}
	dataMap := map[string]any{
		"peer_id": peerID,
		"ok":      true,
	}
	if selectedPath != "" {
		facts = append(facts, poc.Fact{Message: "selected_path=" + selectedPath})
		lines = append(lines, "selected_path="+selectedPath)
		dataMap["selected_path"] = selectedPath
	}
	lines = append(lines, "ping=ok")
	data := mustJSONMarshal(dataMap)
	report := markdownReport("ping", facts, nil)
	return r.successResult(lines, facts, nil, data, report), nil
}

func (r *Runtime) doShellList(ctx context.Context, args ShellArgs) (ActionResult, *problem) {
	peerID := strings.TrimSpace(args.PeerID)
	if peerID == "" {
		return ActionResult{}, newProblem(
			StageShell,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"missing peer_id",
			nil,
			[]poc.Suggestion{{Message: "use: miopunch sh ls <peer_id> [target] [--ready]"}},
		)
	}
	target := strings.TrimSpace(args.Target)
	if args.ReadyOnly && target != "" {
		return ActionResult{}, newProblem(
			StageShell,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"--ready cannot be combined with a concrete target",
			nil,
			[]poc.Suggestion{
				{Message: "use: miopunch sh ls <peer_id> --ready"},
				{Message: "or: miopunch sh ls <peer_id> <target>"},
			},
		)
	}
	policy, problem := normalizePeerPathPolicy(args.P2PNetwork, args.P2PIPFamily)
	if problem != nil {
		return ActionResult{}, problem
	}
	sess, problem := r.ensurePeerSession(ctx, peerID, policy)
	if problem != nil {
		return ActionResult{}, problem
	}
	reply, problem := r.exchangeShellControl(ctx, sess, shellproto.Control{
		Op:        shellproto.OpShLS,
		Target:    target,
		ReadyOnly: args.ReadyOnly,
	})
	if problem != nil {
		r.closePeerSessionOnTransportProblem(sess, problem)
		return ActionResult{}, appendSessionPathProblemFacts(problem, sess)
	}

	lines := []string{}
	if strings.TrimSpace(reply.Target) == "" {
		lines = append(lines, reply.Targets...)
	} else {
		lines = append(lines, reply.Sessions...)
	}
	selectedPath := selectedPathFromSession(sess)
	facts := []poc.Fact{{Message: "peer_id=" + peerID}}
	if selectedPath != "" {
		facts = append(facts, poc.Fact{Message: "selected_path=" + selectedPath})
	}
	if strings.TrimSpace(reply.Target) == "" {
		if args.ReadyOnly {
			readyCount, unsupportedCount, unknownCount := readyStatusCounts(reply.TargetStatuses)
			targetCount := len(reply.TargetStatuses)
			if targetCount == 0 {
				targetCount = len(reply.Targets)
			}
			for _, status := range reply.TargetStatuses {
				if fact := readinessFact(status); fact != "" {
					facts = append(facts, poc.Fact{Message: fact})
				}
				if detail := readinessDetailFact(status); detail != "" {
					facts = append(facts, poc.Fact{Message: detail})
				}
			}
			facts = append(facts,
				poc.Fact{Message: fmt.Sprintf("target_count=%d", targetCount)},
				poc.Fact{Message: fmt.Sprintf("ready_target_count=%d", readyCount)},
				poc.Fact{Message: fmt.Sprintf("unsupported_target_count=%d", unsupportedCount)},
				poc.Fact{Message: fmt.Sprintf("unknown_target_count=%d", unknownCount)},
			)
		} else {
			for _, target := range reply.Targets {
				target = strings.TrimSpace(target)
				if target == "" {
					continue
				}
				facts = append(facts, poc.Fact{Message: "target=" + target})
			}
			facts = append(facts, poc.Fact{Message: fmt.Sprintf("target_count=%d", len(reply.Targets))})
		}
	} else {
		for _, sessionName := range reply.Sessions {
			sessionName = strings.TrimSpace(sessionName)
			if sessionName == "" {
				continue
			}
			facts = append(facts, poc.Fact{Message: "session=" + sessionName})
		}
	}
	dataMap := map[string]any{
		"peer_id":  peerID,
		"target":   reply.Target,
		"targets":  reply.Targets,
		"sessions": reply.Sessions,
	}
	if selectedPath != "" {
		dataMap["selected_path"] = selectedPath
	}
	if args.ReadyOnly {
		dataMap["target_statuses"] = reply.TargetStatuses
	}
	data := mustJSONMarshal(dataMap)
	if strings.TrimSpace(reply.Target) != "" {
		facts = append(facts, poc.Fact{Message: "target=" + reply.Target})
		facts = append(facts, poc.Fact{Message: fmt.Sprintf("session_count=%d", len(reply.Sessions))})
	}
	report := markdownReport("sh ls", facts, nil)
	return r.successResult(lines, facts, nil, data, report), nil
}

func readyStatusCounts(statuses []shellproto.TargetStatus) (int, int, int) {
	var readyCount int
	var unsupportedCount int
	var unknownCount int
	for _, status := range statuses {
		switch strings.TrimSpace(status.Status) {
		case shelltarget.TargetStatusReady:
			readyCount++
		case shelltarget.TargetStatusUnsupported:
			unsupportedCount++
		case shelltarget.TargetStatusUnknown:
			unknownCount++
		}
	}
	return readyCount, unsupportedCount, unknownCount
}

func readinessFact(status shellproto.TargetStatus) string {
	target := strings.TrimSpace(status.Target)
	if target == "" {
		return ""
	}
	result := "target=" + target
	if value := strings.TrimSpace(status.Status); value != "" {
		result += " status=" + value
	}
	if value := strings.TrimSpace(status.ReasonCode); value != "" {
		result += " reason_code=" + value
	}
	return result
}

func readinessDetailFact(status shellproto.TargetStatus) string {
	target := strings.TrimSpace(status.Target)
	message := strings.TrimSpace(status.Message)
	if target == "" || message == "" {
		return ""
	}
	return "target=" + target + " detail=" + message
}

func (r *Runtime) doShell(ctx context.Context, args ShellArgs) (ActionResult, *problem) {
	peerID := strings.TrimSpace(args.PeerID)
	if peerID == "" {
		return ActionResult{}, newProblem(
			StageShell,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"missing peer_id",
			nil,
			[]poc.Suggestion{{Message: "use: miopunch sh <peer_id> [target] [-s session]"}},
		)
	}
	policy, problem := normalizePeerPathPolicy(args.P2PNetwork, args.P2PIPFamily)
	if problem != nil {
		return ActionResult{}, problem
	}
	sess, problem := r.ensurePeerSession(ctx, peerID, policy)
	if problem != nil {
		return ActionResult{}, problem
	}
	if problem := r.exchangePing(ctx, sess, peerID); problem != nil {
		r.closePeerSessionOnTransportProblem(sess, problem)
		return ActionResult{}, appendSessionPathProblemFacts(problem, sess)
	}

	target := strings.TrimSpace(args.Target)
	sessionName := defaultShellSessionName(args.Session)
	stream, reply, problem := r.openRemoteShellAttach(ctx, sess, target, sessionName)
	if problem != nil {
		r.closePeerSessionOnTransportProblem(sess, problem)
		return ActionResult{}, appendSessionPathProblemFacts(problem, sess)
	}

	shellSessionID, err := wire.NewMsgID()
	if err != nil {
		_ = stream.Close()
		return ActionResult{}, appendSessionPathProblemFacts(
			wrapProblem(StageShell, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to allocate shell session id", err, "retry"),
			sess,
		)
	}
	nowUnixMs := time.Now().UTC().UnixMilli()
	r.mu.Lock()
	r.shellSessions[shellSessionID] = &shellSessionState{
		summary: ShellSession{
			ID:              shellSessionID,
			PeerID:          peerID,
			Target:          reply.Target,
			Session:         reply.Session,
			Status:          "pending",
			CreatedAtUnixMs: nowUnixMs,
		},
		stream: stream,
	}
	r.mu.Unlock()
	r.notifyChange("snapshot.updated")

	selectedPath := selectedPathFromSession(sess)
	facts := []poc.Fact{
		{Message: "peer_id=" + peerID},
		{Message: "shell_session_id=" + shellSessionID},
		{Message: "target=" + reply.Target},
		{Message: "session=" + reply.Session},
	}
	dataMap := map[string]any{
		"peer_id":          peerID,
		"shell_session_id": shellSessionID,
		"target":           reply.Target,
		"session":          reply.Session,
	}
	if selectedPath != "" {
		facts = append(facts, poc.Fact{Message: "selected_path=" + selectedPath})
		dataMap["selected_path"] = selectedPath
	}
	data := mustJSONMarshal(dataMap)
	report := markdownReport("sh", facts, nil)
	result := r.successResult(nil, facts, nil, data, report)
	result.ShellSessionID = shellSessionID
	return result, nil
}

func (r *Runtime) doRevoke(ctx context.Context, args RevokeArgs) (ActionResult, *problem) {
	_, networkID, _, _, _, metaProblem := r.requireAdminAuthority()
	if metaProblem != nil {
		return ActionResult{}, metaProblem
	}
	if !args.Dangerous {
		return ActionResult{}, newProblem(
			StageEnroll,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"missing --dangerous (revoke is irreversible in the POC runtime)",
			nil,
			[]poc.Suggestion{{Message: "re-run with: miopunch revoke <peer_id> --dangerous"}},
		)
	}
	peerID := strings.TrimSpace(args.PeerID)
	if peerID == "" {
		return ActionResult{}, newProblem(
			StageEnroll,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"missing peer_id",
			nil,
			[]poc.Suggestion{{Message: "use: miopunch revoke <peer_id> --dangerous"}},
		)
	}

	roster, err := r.store.LoadRosterSnapshot(networkID)
	if err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to load roster snapshot", err, "retry")
	}
	filtered := make([]persist.RosterEntry, 0, len(roster.Entries))
	removed := false
	for _, entry := range roster.Entries {
		if entry.PeerID == peerID {
			removed = true
			continue
		}
		filtered = append(filtered, entry)
	}
	if !removed {
		return ActionResult{}, newProblem(
			StageEnroll,
			poc.ReasonCodeNotFound,
			poc.ExitCodeNotFound,
			"peer is not present in the roster",
			[]poc.Fact{{Message: "peer_id=" + peerID}},
			[]poc.Suggestion{{Message: "run: miopunch ls"}},
		)
	}
	if err := r.store.ReplaceRosterSnapshot(networkID, persist.RosterSnapshot{Entries: filtered}); err != nil {
		return ActionResult{}, wrapProblem(StageEnroll, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to persist revoked roster snapshot", err, "retry")
	}
	if sess, ok := r.peerSessions.Find(dataplane.SessionKey{RemotePeerID: peerID}); ok {
		r.peerSessions.Close(sess.Key(), dataplane.CloseReasonAuthorizationRevocation)
	}
	if err := r.refreshPresenceRoster(ctx); err != nil {
		return ActionResult{}, wrapProblem(StageDiscover, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to refresh presence roster", err, "retry")
	}
	r.mu.Lock()
	delete(r.pingGate, peerID)
	r.mu.Unlock()
	r.notifyChange("snapshot.updated")

	facts := []poc.Fact{{Message: "revoked_peer_id=" + peerID}}
	lines := []string{"revoked_peer_id=" + peerID}
	data := mustJSONMarshal(map[string]any{
		"peer_id": peerID,
		"revoked": true,
	})
	report := markdownReport("revoke", facts, nil)
	return r.successResult(lines, facts, nil, data, report), nil
}

func (r *Runtime) successResult(lines []string, facts []poc.Fact, suggestions []poc.Suggestion, data json.RawMessage, report string) ActionResult {
	snapshot := r.Snapshot()
	return ActionResult{
		Stage:          snapshot.Stage,
		ReasonCode:     poc.ReasonCodeOK,
		ExitCode:       poc.ExitCodeOK,
		Summary:        snapshot.Summary,
		Evidence:       Evidence{Facts: cloneFacts(facts), Suggestions: cloneSuggestions(suggestions)},
		Snapshot:       snapshot,
		Lines:          append([]string(nil), lines...),
		ReportMarkdown: report,
		Data:           data,
	}
}

func (r *Runtime) failureResult(problem *problem) ActionResult {
	snapshot := r.Snapshot()
	return ActionResult{
		Stage:      problem.stage,
		ReasonCode: problem.reasonCode,
		ExitCode:   problem.exitCode,
		Summary:    snapshot.Summary,
		Evidence: Evidence{
			Facts:       cloneFacts(problem.facts),
			Suggestions: cloneSuggestions(problem.suggestions),
		},
		Snapshot: snapshot,
	}
}

func (r *Runtime) requireAdminAuthority() (persist.DeviceKeys, string, ed25519.PrivateKey, ed25519.PublicKey, []byte, *problem) {
	deviceKeys, err := r.store.LoadDeviceKeys()
	if err != nil {
		return persist.DeviceKeys{}, "", nil, nil, nil, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to load device keys", err, "retry")
	}
	localPriv, err := deviceKeys.Ed25519PrivateKey()
	if err != nil {
		return persist.DeviceKeys{}, "", nil, nil, nil, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to derive local signing key", err, "retry")
	}
	r.mu.Lock()
	meta := r.meta
	r.mu.Unlock()
	if strings.TrimSpace(meta.ActiveNetworkID) == "" {
		return persist.DeviceKeys{}, "", nil, nil, nil, newProblem(
			StageNetwork,
			poc.ReasonCodeNotFound,
			poc.ExitCodeNotFound,
			"no active network is joined",
			nil,
			[]poc.Suggestion{
				{Message: "run: miopunch init-network"},
				{Message: "or: miopunch join <invite_code>"},
			},
		)
	}
	if strings.TrimSpace(meta.Role) != "admin" {
		return persist.DeviceKeys{}, "", nil, nil, nil, newProblem(
			StageNetwork,
			poc.ReasonCodeForbidden,
			poc.ExitCodeForbidden,
			"this runtime is not the network admin",
			nil,
			[]poc.Suggestion{{Message: "run the command from the admin daemon state"}},
		)
	}
	authorityPub, err := meta.authorityEd25519()
	if err != nil {
		return persist.DeviceKeys{}, "", nil, nil, nil, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to decode authority public key", err, "retry")
	}
	authorityX25519Pub, err := meta.authorityX25519()
	if err != nil {
		return persist.DeviceKeys{}, "", nil, nil, nil, wrapProblem(StageNetwork, poc.ReasonCodeInternal, poc.ExitCodeInternal, "failed to decode authority x25519 public key", err, "retry")
	}
	return deviceKeys, meta.ActiveNetworkID, localPriv, authorityPub, authorityX25519Pub, nil
}

func (r *Runtime) ensurePeerSession(ctx context.Context, peerID string, policy peerPathPolicy) (session.PeerSession, *problem) {
	if err := r.ensureWorkers(ctx); err != nil {
		return nil, wrapProblem(StageDiscover, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to start runtime workers", err, "retry")
	}
	if existing, ok := r.peerSessions.Find(dataplane.SessionKey{RemotePeerID: peerID}); ok {
		if policy.matchesSession(existing) {
			return existing, nil
		}
		if policy.explicit() {
			logutil.Debugf(
				"runtime peer session policy mismatch: peer_id=%s requested_p2p_network=%s requested_p2p_ip_family=%s existing_key=%+v existing_selected_path=%s",
				peerID,
				policy.P2PNetwork,
				policy.P2PIPFamily,
				existing.Key().Normalize(),
				selectedPathFromSession(existing),
			)
			r.peerSessions.CloseIfMatch(existing, dataplane.CloseReasonSessionSuperseded)
		}
	}
	if problem := policy.unsupportedTCPOnlyProblem(); problem != nil {
		return nil, problem
	}

	r.mu.Lock()
	presenceSession := r.presence
	r.mu.Unlock()
	if presenceSession != nil {
		waitCtx, cancel := withDefaultTimeout(ctx, 5*time.Second)
		defer cancel()
		_, _ = presenceSession.WaitForPeerState(waitCtx, peerID, presence.OnlineStateOnline)
	}

	punchCfg, problem := r.punchConfig(policy)
	if problem != nil {
		return nil, problem
	}
	logutil.Debugf(
		"runtime peer session dial start: peer_id=%s p2p_network=%s p2p_ip_family=%s",
		peerID,
		policy.P2PNetwork,
		policy.P2PIPFamily,
	)
	result, err := punch.Dial(ctx, punchCfg, punch.Target{PeerID: peerID})
	if err != nil {
		return nil, punchProblem("failed to establish punched path", peerID, err)
	}
	sess, err := session.Dial(ctx, r.sessionConfig(), result)
	if err != nil {
		_ = result.Close()
		return nil, appendPathResultProblemFacts(
			wrapProblem(StageSecureSession, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to establish secure session", err, "retry"),
			result,
		)
	}
	logutil.Debugf(
		"runtime peer session dial selected: peer_id=%s p2p_network=%s p2p_ip_family=%s selected_path=%s remote_udp=%s",
		peerID,
		policy.P2PNetwork,
		policy.P2PIPFamily,
		result.Evidence.SelectedPath,
		result.Evidence.SelectedRemoteUDP,
	)
	r.registerPeerSession(peerID, sess)
	return sess, nil
}

func (r *Runtime) exchangePing(ctx context.Context, sess session.PeerSession, peerID string) *problem {
	reply, problem := r.exchangeShellControl(ctx, sess, shellproto.Control{Op: shellproto.OpPing})
	if problem != nil {
		return problem
	}
	if !reply.OK {
		return newProblem(
			StageShell,
			poc.ReasonCodeUnavailable,
			poc.ExitCodeUnavailable,
			"ping gate was rejected by the remote peer",
			nil,
			[]poc.Suggestion{{Message: "retry"}},
		)
	}
	r.markPingGate(peerID)
	return nil
}

func (r *Runtime) exchangeShellControl(ctx context.Context, sess session.PeerSession, control shellproto.Control) (shellproto.Control, *problem) {
	stream, problem := r.openShellStream(ctx, sess)
	if problem != nil {
		return shellproto.Control{}, problem
	}
	defer stream.Close()

	if err := shellproto.WriteJSON(stream, control); err != nil {
		return shellproto.Control{}, wrapProblem(StageShell, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to send shell control request", err, "retry")
	}
	var reply shellproto.Control
	if err := shellproto.ReadJSON(stream, &reply); err != nil {
		return shellproto.Control{}, wrapProblem(StageShell, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to read shell control response", err, "retry")
	}
	if reply.Error != nil {
		return shellproto.Control{}, controlProblem(StageShell, reply.Error)
	}
	return reply, nil
}

func (r *Runtime) openRemoteShellAttach(ctx context.Context, sess session.PeerSession, target string, sessionName string) (io.ReadWriteCloser, shellproto.Control, *problem) {
	stream, problem := r.openShellStream(ctx, sess)
	if problem != nil {
		return nil, shellproto.Control{}, problem
	}

	request := shellproto.Control{
		Op:      shellproto.OpShAttach,
		Target:  strings.TrimSpace(target),
		Session: defaultShellSessionName(sessionName),
	}
	if err := shellproto.WriteJSON(stream, request); err != nil {
		_ = stream.Close()
		return nil, shellproto.Control{}, wrapProblem(StageShell, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to send shell attach request", err, "retry")
	}
	var reply shellproto.Control
	if err := shellproto.ReadJSON(stream, &reply); err != nil {
		_ = stream.Close()
		return nil, shellproto.Control{}, wrapProblem(StageShell, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to read shell attach response", err, "retry")
	}
	if reply.Error != nil {
		_ = stream.Close()
		return nil, shellproto.Control{}, controlProblem(StageShell, reply.Error)
	}
	return stream, reply, nil
}

func (r *Runtime) openShellStream(ctx context.Context, sess session.PeerSession) (io.ReadWriteCloser, *problem) {
	stream, err := session.OpenStream(ctx, sess, dataplane.StreamOpen{
		Kind: dataplane.StreamKindShellV0,
	})
	if err != nil {
		return nil, wrapProblem(StageShell, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "failed to open shell stream", err, "retry")
	}
	return stream, nil
}

func (r *Runtime) closePeerSessionOnTransportProblem(sess session.PeerSession, problem *problem) {
	if r == nil || sess == nil || !isPeerSessionTransportProblem(problem) {
		return
	}
	r.peerSessions.CloseIfMatch(sess, dataplane.CloseReasonTransportFatal)
}

func isPeerSessionTransportProblem(problem *problem) bool {
	if problem == nil {
		return false
	}
	switch problem.reasonCode {
	case poc.ReasonCodeUnavailable, poc.ReasonCodeTimeout:
	default:
		return false
	}
	if problem.stage == StageSecureSession {
		return true
	}
	if problem.stage != StageShell {
		return false
	}
	message := strings.TrimSpace(problem.message)
	return strings.HasPrefix(message, "failed to open shell stream") ||
		strings.HasPrefix(message, "failed to send shell control request") ||
		strings.HasPrefix(message, "failed to read shell control response") ||
		strings.HasPrefix(message, "failed to send shell attach request") ||
		strings.HasPrefix(message, "failed to read shell attach response")
}

func (r *Runtime) currentBrokerEndpoint() string {
	r.mu.Lock()
	defer r.mu.Unlock()
	if strings.TrimSpace(r.meta.RuntimeBrokerOverride) != "" {
		return strings.TrimSpace(r.meta.RuntimeBrokerOverride)
	}
	if r.broker != nil {
		return r.broker.Endpoint()
	}
	return strings.TrimSpace(r.meta.BrokerEndpoint)
}

func joinTopic(scope persist.TopicScope) string {
	return fmt.Sprintf("mp/v1/join/%s", strings.ToLower(strings.TrimSpace(scope.NetRoot)))
}

func enrollRosterFromPersist(snapshot persist.RosterSnapshot) (enroll.RosterSnapshot, error) {
	out := enroll.RosterSnapshot{Entries: make([]enroll.RosterEntry, 0, len(snapshot.Entries))}
	for _, entry := range snapshot.Entries {
		credential, err := enroll.UnmarshalMemberCredential(entry.MemberCredential)
		if err != nil {
			return enroll.RosterSnapshot{}, err
		}
		out.Entries = append(out.Entries, enroll.RosterEntry{
			PeerID:           entry.PeerID,
			MemberCredential: credential,
			DeviceName:       entry.DeviceName,
			Platform:         entry.Platform,
		})
	}
	return out, nil
}

func upsertEnrollRosterEntry(entries []enroll.RosterEntry, entry enroll.RosterEntry) []enroll.RosterEntry {
	out := make([]enroll.RosterEntry, 0, len(entries)+1)
	replaced := false
	for _, current := range entries {
		if current.PeerID == entry.PeerID {
			out = append(out, entry)
			replaced = true
			continue
		}
		out = append(out, current)
	}
	if !replaced {
		out = append(out, entry)
	}
	sort.Slice(out, func(i, j int) bool {
		return out[i].PeerID < out[j].PeerID
	})
	return out
}

func controlProblem(stage Stage, controlErr *shellproto.ControlError) *problem {
	if controlErr == nil {
		return nil
	}
	reasonCode := poc.ReasonCodeUnavailable
	switch strings.TrimSpace(controlErr.ReasonCode) {
	case shellproto.ReasonHelloRequired:
		reasonCode = poc.ReasonCodeForbidden
	case string(poc.ReasonCodeSHTargetNotFound):
		reasonCode = poc.ReasonCodeSHTargetNotFound
	case string(poc.ReasonCodeSHTargetAmbiguous):
		reasonCode = poc.ReasonCodeSHTargetAmbiguous
	case string(poc.ReasonCodeSHTmuxMissing):
		reasonCode = poc.ReasonCodeSHTmuxMissing
	}
	suggestions := make([]poc.Suggestion, 0, len(controlErr.Suggestions))
	for _, suggestion := range controlErr.Suggestions {
		if strings.TrimSpace(suggestion) == "" {
			continue
		}
		suggestions = append(suggestions, poc.Suggestion{Message: suggestion})
	}
	return newProblem(stage, reasonCode, exitCodeForReason(reasonCode), strings.TrimSpace(controlErr.Message), nil, suggestions)
}

func punchProblem(message string, peerID string, err error) *problem {
	reasonCode := poc.ReasonCodeUnavailable
	exitCode := poc.ExitCodeUnavailable
	if errors.Is(err, punch.ErrAttemptBudgetExceeded) || errors.Is(err, context.DeadlineExceeded) {
		reasonCode = poc.ReasonCodeTimeout
		exitCode = poc.ExitCodeTimeout
	}
	facts := []poc.Fact{
		{Message: message},
		{Message: "peer_id=" + strings.TrimSpace(peerID)},
	}
	var diagnosticErr *punch.Error
	if errors.As(err, &diagnosticErr) {
		facts = append(facts, diagnosticErr.Diagnostic.Facts()...)
	}
	if err != nil {
		facts = append(facts, poc.Fact{Message: "error=" + err.Error()})
	}
	return newProblem(
		StagePunch,
		reasonCode,
		exitCode,
		strings.TrimSpace(message),
		facts,
		[]poc.Suggestion{{Message: "check that the target peer is online and retry"}},
	)
}

func exitCodeForReason(reasonCode poc.ReasonCode) poc.ExitCode {
	switch reasonCode {
	case poc.ReasonCodeBadRequest:
		return poc.ExitCodeBadRequest
	case poc.ReasonCodeForbidden:
		return poc.ExitCodeForbidden
	case poc.ReasonCodeConflict:
		return poc.ExitCodeConflict
	case poc.ReasonCodeNotFound, poc.ReasonCodeSHTargetNotFound:
		return poc.ExitCodeNotFound
	default:
		return poc.ExitCodeUnavailable
	}
}

func markdownReport(title string, facts []poc.Fact, suggestions []poc.Suggestion) string {
	var builder strings.Builder
	builder.WriteString("# ")
	builder.WriteString(strings.TrimSpace(title))
	builder.WriteString("\n\n")
	if len(facts) > 0 {
		builder.WriteString("## Facts\n\n")
		for _, fact := range facts {
			if strings.TrimSpace(fact.Message) == "" {
				continue
			}
			builder.WriteString("- ")
			builder.WriteString(fact.Message)
			builder.WriteString("\n")
		}
		builder.WriteString("\n")
	}
	if len(suggestions) > 0 {
		builder.WriteString("## Suggestions\n\n")
		for _, suggestion := range suggestions {
			if strings.TrimSpace(suggestion.Message) == "" {
				continue
			}
			builder.WriteString("- ")
			builder.WriteString(suggestion.Message)
			builder.WriteString("\n")
		}
	}
	return builder.String()
}

func mustJSONMarshal(value any) json.RawMessage {
	if value == nil {
		return nil
	}
	data, err := json.Marshal(value)
	if err != nil {
		return nil
	}
	return data
}

func withDefaultTimeout(ctx context.Context, timeout time.Duration) (context.Context, context.CancelFunc) {
	if ctx == nil {
		ctx = context.Background()
	}
	if _, ok := ctx.Deadline(); ok || timeout <= 0 {
		return ctx, func() {}
	}
	return context.WithTimeout(ctx, timeout)
}
