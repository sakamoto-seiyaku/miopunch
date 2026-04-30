package task

import (
	"context"
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"slices"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

const (
	bootstrapMoreRequestKindV0  = "bootstrap_more_request"
	bootstrapMoreResponseKindV0 = "bootstrap_more_response"

	defaultBootstrapMoreRequestTimeout = 5 * time.Second
	defaultBootstrapMoreRespondTimeout = 15 * time.Second
	maxBootstrapMoreRounds             = 2
	maxBootstrapMoreCandidates         = 2
)

type bootstrapMoreFailureV0 struct {
	PeerID string `json:"peer_id"`
	Reason string `json:"reason,omitempty"`
}

type bootstrapMoreRequestBodyV0 struct {
	Round            int                      `json:"round"`
	AttemptedPeerIDs []string                 `json:"attempted_peer_ids,omitempty"`
	Failures         []bootstrapMoreFailureV0 `json:"failures,omitempty"`
}

type bootstrapMoreResponseBodyV0 struct {
	Round         int                                `json:"round"`
	Candidates    []pocstate.BootstrapPeerEvidenceV0 `json:"candidates"`
	StopCondition string                             `json:"stop_condition"`
}

func (m *Manager) runBootstrapMoreTask(taskID string, rawArgs []byte) {
	var args BootstrapMoreArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	args = args.normalize()
	if args.Mode == "" {
		args.Mode = "request"
	}
	if args.Round == 0 {
		args.Round = 1
	}
	if args.Round < 1 || args.Round > maxBootstrapMoreRounds {
		m.addFact(taskID, poc.Fact{Message: "bootstrap_more round out of range"})
		m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("max_rounds=%d", maxBootstrapMoreRounds)})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	switch args.Mode {
	case "request":
		m.runBootstrapMoreRequestTask(taskID, args)
	case "respond_once":
		m.runBootstrapMoreRespondOnceTask(taskID, args)
	default:
		m.addFact(taskID, poc.Fact{Message: "unknown bootstrap_more mode: " + args.Mode})
		m.addSuggestion(taskID, poc.Suggestion{Message: "use: miopunch bootstrap-more [--respond-once]"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
	}
}

func (m *Manager) runBootstrapMoreRequestTask(taskID string, args BootstrapMoreArgs) {
	targetPeerID, err := controlplane.CanonicalizePeerID(args.TargetPeerID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid target_peer_id: " + err.Error()})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	timeout, err := parseBootstrapMoreTimeout(args.Timeout, defaultBootstrapMoreRequestTimeout)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid timeout: " + err.Error()})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	ctx, cancel := context.WithTimeout(m.ctx, timeout)
	defer cancel()

	stateDir, selfID, netState, err := m.bootstrapMoreState()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	attempted, err := canonicalBootstrapMorePeerIDs(args.AttemptedPeerIDs)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid attempted peer: " + err.Error()})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	selfInbox, err := controlplane.DeriveInboxTopic(netState.NetSecret, selfID.PeerID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "derive self inbox: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	targetInbox, err := controlplane.DeriveInboxTopic(netState.NetSecret, targetPeerID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "derive target inbox: " + err.Error()})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	mbs, err := openBootstrapMoreMailboxes(ctx, netState.BrokersEffective, "miopunch-bootstrap-more-request")
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "mqtt connect failed: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	defer closeMailboxes(mbs)

	for _, mb := range mbs {
		if err := mb.Subscribe(ctx, selfInbox); err != nil {
			m.addFact(taskID, poc.Fact{Message: "subscribe self inbox failed: " + err.Error()})
			m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
			return
		}
	}
	evCh, stop := fanInMailboxEvents(ctx, mbs)
	defer stop()

	req, err := newBootstrapMoreRequest(selfID, targetPeerID, args.Round, attempted, timeout)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "build bootstrap_more_request: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	reqPlain, err := controlplane.MarshalMessage(req)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "marshal bootstrap_more_request: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	reqCipher, err := controlplane.SealGroupV0(netState.NetSecret, reqPlain)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "encrypt bootstrap_more_request: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	m.setStage(taskID, poc.StagePeerContact, "send bootstrap_more_request")
	if err := publishToAll(ctx, mbs, targetInbox, reqCipher); err != nil {
		m.addFact(taskID, poc.Fact{Message: "publish bootstrap_more_request failed: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	targetPub, err := peerEd25519Pub(stateDir, targetPeerID, selfID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "resolve target pubkey: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	m.setStage(taskID, poc.StagePeerContact, "wait bootstrap_more_response")
	resp, body, err := waitBootstrapMoreResponse(ctx, evCh, selfInbox, netState.NetSecret, selfID.PeerID, targetPeerID, req.Route.MsgID, targetPub)
	if err != nil {
		if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
			m.addFact(taskID, poc.Fact{Message: "bootstrap_more timed out waiting for response"})
			m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("bootstrap_more_timeout_ms=%d", timeout.Milliseconds())})
			m.done(taskID, poc.ReasonCodeTimeout, poc.ExitCodeTimeout)
			return
		}
		m.addFact(taskID, poc.Fact{Message: "bootstrap_more response failed: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	if err := saveBootstrapMoreResponse(stateDir, body.Candidates); err != nil {
		m.addFact(taskID, poc.Fact{Message: "save bootstrap_more evidence: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	m.addFact(taskID, poc.Fact{TermID: "bootstrap_more_request_id", Message: "bootstrap_more_request_id=" + req.Route.MsgID})
	m.addFact(taskID, poc.Fact{TermID: "bootstrap_more_response_id", Message: "bootstrap_more_response_id=" + resp.Route.MsgID})
	m.addFact(taskID, poc.Fact{TermID: "target_peer_id", Message: "target_peer_id=" + targetPeerID})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("bootstrap_more_round=%d", args.Round)})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("bootstrap_more_timeout_ms=%d", timeout.Milliseconds())})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("bootstrap_more_candidates=%d", len(body.Candidates))})
	m.addFact(taskID, poc.Fact{Message: "bootstrap_more_stop_condition=" + body.StopCondition})
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}

func (m *Manager) runBootstrapMoreRespondOnceTask(taskID string, args BootstrapMoreArgs) {
	timeout, err := parseBootstrapMoreTimeout(args.Timeout, defaultBootstrapMoreRespondTimeout)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid timeout: " + err.Error()})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	ctx, cancel := context.WithTimeout(m.ctx, timeout)
	defer cancel()

	stateDir, selfID, netState, err := m.bootstrapMoreState()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	selfInbox, err := controlplane.DeriveInboxTopic(netState.NetSecret, selfID.PeerID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "derive self inbox: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	mbs, err := openBootstrapMoreMailboxes(ctx, netState.BrokersEffective, "miopunch-bootstrap-more-respond")
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "mqtt connect failed: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}
	defer closeMailboxes(mbs)

	for _, mb := range mbs {
		if err := mb.Subscribe(ctx, selfInbox); err != nil {
			m.addFact(taskID, poc.Fact{Message: "subscribe self inbox failed: " + err.Error()})
			m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
			return
		}
	}
	evCh, stop := fanInMailboxEvents(ctx, mbs)
	defer stop()

	m.setStage(taskID, poc.StagePeerContact, "wait bootstrap_more_request")
	req, body, err := waitBootstrapMoreRequest(ctx, evCh, selfInbox, netState.NetSecret, selfID.PeerID, stateDir, selfID)
	if err != nil {
		if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
			m.addFact(taskID, poc.Fact{Message: "bootstrap_more responder timed out waiting for request"})
			m.done(taskID, poc.ReasonCodeTimeout, poc.ExitCodeTimeout)
			return
		}
		m.addFact(taskID, poc.Fact{Message: "bootstrap_more request failed: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	st, err := m.loadState()
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "load state: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	candidates := bootstrapMoreCandidates(selfID.PeerID, req.Signed.SenderPeerID, body.AttemptedPeerIDs, st)
	stopCondition := "ok"
	if len(candidates) == 0 {
		stopCondition = "exhausted_candidates"
	}
	resp, respBody, err := newBootstrapMoreResponse(selfID, req, body.Round, candidates, stopCondition)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "build bootstrap_more_response: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	respPlain, err := controlplane.MarshalMessage(resp)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "marshal bootstrap_more_response: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}
	respCipher, err := controlplane.SealGroupV0(netState.NetSecret, respPlain)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "encrypt bootstrap_more_response: " + err.Error()})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	replyInbox, err := controlplane.DeriveInboxTopic(netState.NetSecret, req.Signed.SenderPeerID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "derive requester inbox: " + err.Error()})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	m.setStage(taskID, poc.StagePeerContact, "send bootstrap_more_response")
	if err := publishToAll(ctx, mbs, replyInbox, respCipher); err != nil {
		m.addFact(taskID, poc.Fact{Message: "publish bootstrap_more_response failed: " + err.Error()})
		m.done(taskID, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable)
		return
	}

	m.addFact(taskID, poc.Fact{TermID: "bootstrap_more_request_id", Message: "bootstrap_more_request_id=" + req.Route.MsgID})
	m.addFact(taskID, poc.Fact{TermID: "bootstrap_more_response_id", Message: "bootstrap_more_response_id=" + resp.Route.MsgID})
	m.addFact(taskID, poc.Fact{TermID: "requester_peer_id", Message: "requester_peer_id=" + req.Signed.SenderPeerID})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("bootstrap_more_round=%d", body.Round)})
	m.addFact(taskID, poc.Fact{Message: fmt.Sprintf("bootstrap_more_candidates=%d", len(respBody.Candidates))})
	m.addFact(taskID, poc.Fact{Message: "bootstrap_more_stop_condition=" + respBody.StopCondition})
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}

func (m *Manager) bootstrapMoreState() (string, pocstate.Identity, pocstate.Net, error) {
	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		return "", pocstate.Identity{}, pocstate.Net{}, fmt.Errorf("state_dir: %w", err)
	}
	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		return "", pocstate.Identity{}, pocstate.Net{}, fmt.Errorf("ensure identity: %w", err)
	}
	netState, err := pocstate.LoadNet(stateDir)
	if err != nil {
		return "", pocstate.Identity{}, pocstate.Net{}, fmt.Errorf("load net: %w", err)
	}
	if len(netState.NetSecret) == 0 {
		return "", pocstate.Identity{}, pocstate.Net{}, errors.New("missing net secret")
	}
	if len(netState.BrokersEffective) == 0 {
		return "", pocstate.Identity{}, pocstate.Net{}, errors.New("missing effective brokers")
	}
	return stateDir, selfID, netState, nil
}

func parseBootstrapMoreTimeout(raw string, fallback time.Duration) (time.Duration, error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return fallback, nil
	}
	d, err := time.ParseDuration(raw)
	if err != nil {
		return 0, err
	}
	if d <= 0 {
		return 0, errors.New("timeout must be positive")
	}
	return d, nil
}

func canonicalBootstrapMorePeerIDs(in []string) ([]string, error) {
	out := make([]string, 0, len(in))
	seen := make(map[string]struct{}, len(in))
	for _, raw := range in {
		peerID, err := controlplane.CanonicalizePeerID(raw)
		if err != nil {
			return nil, err
		}
		if _, ok := seen[peerID]; ok {
			continue
		}
		seen[peerID] = struct{}{}
		out = append(out, peerID)
	}
	return out, nil
}

func openBootstrapMoreMailboxes(ctx context.Context, brokers []string, clientIDPrefix string) ([]*mqttMailbox, error) {
	mbs := make([]*mqttMailbox, 0, len(brokers))
	for _, ep := range brokers {
		mb, err := openMQTTMailbox(ctx, ep, clientIDPrefix)
		if err != nil {
			closeMailboxes(mbs)
			return nil, err
		}
		mbs = append(mbs, mb)
	}
	if len(mbs) == 0 {
		return nil, errors.New("no broker mailboxes opened")
	}
	return mbs, nil
}

func closeMailboxes(mbs []*mqttMailbox) {
	for _, mb := range mbs {
		_ = mb.Close()
	}
}

func newBootstrapMoreRequest(selfID pocstate.Identity, targetPeerID string, round int, attempted []string, timeout time.Duration) (controlplane.Message, error) {
	body := bootstrapMoreRequestBodyV0{
		Round:            round,
		AttemptedPeerIDs: append([]string(nil), attempted...),
	}
	for _, peerID := range attempted {
		body.Failures = append(body.Failures, bootstrapMoreFailureV0{
			PeerID: peerID,
			Reason: "candidate_exhausted",
		})
	}
	bodyJSON, err := json.Marshal(body)
	if err != nil {
		return controlplane.Message{}, fmt.Errorf("marshal request body: %w", err)
	}
	msgID, err := controlplane.NewMsgID()
	if err != nil {
		return controlplane.Message{}, err
	}
	now := time.Now().UTC()
	msg := controlplane.Message{
		ProtoVersion: controlplane.ProtoVersionV0,
		Route: controlplane.Route{
			DstPeerID:       targetPeerID,
			MsgID:           msgID,
			HopLimit:        0,
			CreatedAtUnixMs: now.UnixMilli(),
			ExpiresAtUnixMs: now.Add(timeout).UnixMilli(),
		},
		Signed: controlplane.Signed{
			SenderPeerID: selfID.PeerID,
			Kind:         bootstrapMoreRequestKindV0,
			Body:         bodyJSON,
		},
	}
	if err := controlplane.SignV0(selfID.Ed25519Priv, &msg); err != nil {
		return controlplane.Message{}, err
	}
	return msg, nil
}

func newBootstrapMoreResponse(selfID pocstate.Identity, req controlplane.Message, round int, candidates []pocstate.BootstrapPeerEvidenceV0, stopCondition string) (controlplane.Message, bootstrapMoreResponseBodyV0, error) {
	body := bootstrapMoreResponseBodyV0{
		Round:         round,
		Candidates:    append([]pocstate.BootstrapPeerEvidenceV0(nil), candidates...),
		StopCondition: stopCondition,
	}
	bodyJSON, err := json.Marshal(body)
	if err != nil {
		return controlplane.Message{}, bootstrapMoreResponseBodyV0{}, fmt.Errorf("marshal response body: %w", err)
	}
	msgID, err := controlplane.NewMsgID()
	if err != nil {
		return controlplane.Message{}, bootstrapMoreResponseBodyV0{}, err
	}
	now := time.Now().UTC()
	msg := controlplane.Message{
		ProtoVersion: controlplane.ProtoVersionV0,
		Route: controlplane.Route{
			DstPeerID:       req.Signed.SenderPeerID,
			MsgID:           msgID,
			HopLimit:        0,
			CreatedAtUnixMs: now.UnixMilli(),
			ExpiresAtUnixMs: req.Route.ExpiresAtUnixMs,
		},
		Signed: controlplane.Signed{
			SenderPeerID: selfID.PeerID,
			Kind:         bootstrapMoreResponseKindV0,
			InReplyTo:    req.Route.MsgID,
			Body:         bodyJSON,
		},
	}
	if err := controlplane.SignV0(selfID.Ed25519Priv, &msg); err != nil {
		return controlplane.Message{}, bootstrapMoreResponseBodyV0{}, err
	}
	return msg, body, nil
}

func waitBootstrapMoreResponse(ctx context.Context, evCh <-chan mailboxEvent, inboxTopic string, netSecret []byte, selfPeerID string, targetPeerID string, requestID string, targetPub ed25519.PublicKey) (controlplane.Message, bootstrapMoreResponseBodyV0, error) {
	for {
		select {
		case <-ctx.Done():
			return controlplane.Message{}, bootstrapMoreResponseBodyV0{}, ctx.Err()
		case ev, ok := <-evCh:
			if !ok {
				return controlplane.Message{}, bootstrapMoreResponseBodyV0{}, errors.New("mailbox closed")
			}
			if ev.Err != nil {
				return controlplane.Message{}, bootstrapMoreResponseBodyV0{}, ev.Err
			}
			if ev.Topic != inboxTopic {
				continue
			}
			msg, err := openBootstrapMoreGroupMessage(netSecret, ev.Payload)
			if err != nil {
				continue
			}
			if msg.Signed.Kind != bootstrapMoreResponseKindV0 || msg.Signed.InReplyTo != requestID {
				continue
			}
			if msg.Signed.SenderPeerID != targetPeerID {
				continue
			}
			if err := controlplane.VerifyV0ForSelf(selfPeerID, targetPub, msg); err != nil {
				continue
			}
			if err := controlplane.ValidateCreatedAtSkew(time.Now().UTC().UnixMilli(), msg.Route.CreatedAtUnixMs); err != nil {
				continue
			}
			var body bootstrapMoreResponseBodyV0
			if err := json.Unmarshal(msg.Signed.Body, &body); err != nil {
				continue
			}
			return msg, body, nil
		}
	}
}

func waitBootstrapMoreRequest(ctx context.Context, evCh <-chan mailboxEvent, inboxTopic string, netSecret []byte, selfPeerID string, stateDir string, selfID pocstate.Identity) (controlplane.Message, bootstrapMoreRequestBodyV0, error) {
	for {
		select {
		case <-ctx.Done():
			return controlplane.Message{}, bootstrapMoreRequestBodyV0{}, ctx.Err()
		case ev, ok := <-evCh:
			if !ok {
				return controlplane.Message{}, bootstrapMoreRequestBodyV0{}, errors.New("mailbox closed")
			}
			if ev.Err != nil {
				return controlplane.Message{}, bootstrapMoreRequestBodyV0{}, ev.Err
			}
			if ev.Topic != inboxTopic {
				continue
			}
			msg, err := openBootstrapMoreGroupMessage(netSecret, ev.Payload)
			if err != nil {
				continue
			}
			if msg.Signed.Kind != bootstrapMoreRequestKindV0 {
				continue
			}
			if msg.Route.DstPeerID != selfPeerID {
				continue
			}
			pub, err := peerEd25519Pub(stateDir, msg.Signed.SenderPeerID, selfID)
			if err != nil {
				continue
			}
			if err := controlplane.VerifyV0ForSelf(selfPeerID, pub, msg); err != nil {
				continue
			}
			if err := controlplane.ValidateRPCRequestTime(time.Now().UTC().UnixMilli(), msg); err != nil {
				continue
			}
			var body bootstrapMoreRequestBodyV0
			if err := json.Unmarshal(msg.Signed.Body, &body); err != nil {
				continue
			}
			return msg, body, nil
		}
	}
}

func openBootstrapMoreGroupMessage(netSecret []byte, payload []byte) (controlplane.Message, error) {
	plain, err := controlplane.OpenGroupV0(netSecret, payload)
	if err != nil {
		return controlplane.Message{}, err
	}
	return controlplane.UnmarshalMessage(plain)
}

func peerEd25519Pub(stateDir string, peerID string, selfID pocstate.Identity) (ed25519.PublicKey, error) {
	peerID = strings.TrimSpace(peerID)
	if peerID == selfID.PeerID {
		return selfID.Ed25519Pub, nil
	}

	if head, err := pocstate.LoadGovernanceHeadSnapshot(stateDir); err == nil {
		if pub, ok, err := head.AdminEd25519Pub(peerID); err != nil {
			return nil, err
		} else if ok {
			return pub, nil
		}
	} else if err != nil && !errors.Is(err, os.ErrNotExist) {
		return nil, err
	}

	decls, err := pocstate.LoadDecls(stateDir)
	if err != nil {
		return nil, err
	}
	for _, decl := range decls.Decls {
		if decl.Kind != pocstate.DeclKindApproveMember {
			continue
		}
		var body pocstate.ApproveMemberBodyV0
		if err := json.Unmarshal(decl.Body, &body); err != nil {
			continue
		}
		if strings.TrimSpace(body.MemberPeerID) != peerID {
			continue
		}
		pub, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(body.Ed25519PubB64))
		if err != nil || len(pub) != ed25519.PublicKeySize {
			continue
		}
		derivedPeerID, err := controlplane.PeerIDFromEd25519Pub(ed25519.PublicKey(pub))
		if err != nil || derivedPeerID != peerID {
			continue
		}
		return ed25519.PublicKey(pub), nil
	}
	return nil, fmt.Errorf("peer public key not found: %s", peerID)
}

func bootstrapMoreCandidates(selfPeerID string, requesterPeerID string, attemptedPeerIDs []string, st pocstate.State) []pocstate.BootstrapPeerEvidenceV0 {
	selfPeerID = strings.TrimSpace(selfPeerID)
	requesterPeerID = strings.TrimSpace(requesterPeerID)
	attempted := make(map[string]struct{}, len(attemptedPeerIDs))
	for _, peerID := range attemptedPeerIDs {
		peerID = strings.TrimSpace(peerID)
		if peerID != "" {
			attempted[peerID] = struct{}{}
		}
	}

	out := make([]pocstate.BootstrapPeerEvidenceV0, 0, maxBootstrapMoreCandidates)
	for _, candidate := range sortedBootstrapCandidates(st.Peers) {
		if len(out) >= maxBootstrapMoreCandidates {
			break
		}
		peerID := candidate.peerID
		if peerID == "" || peerID == selfPeerID || peerID == requesterPeerID {
			continue
		}
		if _, ok := attempted[peerID]; ok {
			continue
		}
		if slices.ContainsFunc(out, func(existing pocstate.BootstrapPeerEvidenceV0) bool {
			return existing.PeerID == peerID
		}) {
			continue
		}
		if _, ok := seedPeerFromPeerConfig(peerID, st.Peers[peerID]); !ok {
			continue
		}
		out = append(out, pocstate.BootstrapPeerEvidenceV0{
			PeerID: peerID,
			Bucket: candidate.bucket,
			Reason: "bootstrap_more_response",
		})
	}
	return out
}

func saveBootstrapMoreResponse(stateDir string, candidates []pocstate.BootstrapPeerEvidenceV0) error {
	bootstrap, err := pocstate.LoadBootstrap(stateDir)
	if err != nil {
		if !errors.Is(err, os.ErrNotExist) {
			return err
		}
		bootstrap = pocstate.BootstrapFileV0{}
	}
	bootstrap.MoreRounds = append(bootstrap.MoreRounds, candidates...)
	return pocstate.SaveBootstrap(stateDir, bootstrap)
}
