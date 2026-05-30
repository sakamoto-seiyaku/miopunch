package runtime

import (
	"context"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	goruntime "runtime"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/buildinfo"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/presence"
	"github.com/miopunch/miopunch/internal/pocv1/punch"
	"github.com/miopunch/miopunch/internal/pocv1/session"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
	"github.com/miopunch/miopunch/internal/shellproto"
	"github.com/miopunch/miopunch/internal/shelltarget"
)

const (
	defaultApprovalWait      = 2 * time.Minute
	defaultInviteLifetime    = 15 * time.Minute
	defaultIdleTimeout       = 2 * time.Minute
	sessionKeepaliveInterval = 30 * time.Second
	sessionKeepaliveMinIdle  = 45 * time.Second
	sessionKeepaliveTimeout  = 15 * time.Second
)

type Options struct {
	Root       string
	DeviceName string
	Platform   string
	AppVersion string
	BrokerURL  string
}

type StatusResponse struct {
	Version   string    `json:"version"`
	StartedAt time.Time `json:"started_at"`
	UptimeMs  int64     `json:"uptime_ms"`
}

type shellSessionState struct {
	summary  ShellSession
	stream   io.ReadWriteCloser
	attached bool
}

type Runtime struct {
	root       string
	store      *persist.Store
	startedAt  time.Time
	deviceName string
	platform   string
	appVersion string

	ctx    context.Context
	cancel context.CancelFunc

	mu                sync.Mutex
	closed            bool
	meta              metadata
	status            *problem
	broker            *embeddedBroker
	presence          *presence.Session
	udpConn           *net.UDPConn
	acceptLoopStarted bool
	keepaliveStarted  bool
	pingGate          map[string]int64
	shellSessions     map[string]*shellSessionState
	subscribers       map[int]chan Event
	nextSubscriberID  int

	wg           sync.WaitGroup
	peerSessions *dataplane.SessionManager
}

func Open(opts Options) (*Runtime, error) {
	root := strings.TrimSpace(opts.Root)
	if root == "" {
		return nil, errors.New("runtime root is required")
	}

	store, err := persist.Open(root)
	if err != nil {
		return nil, err
	}
	if _, err := store.EnsureDeviceKeys(); err != nil {
		return nil, err
	}

	meta, err := loadMetadata(root)
	if err != nil {
		return nil, err
	}

	ctx, cancel := context.WithCancel(context.Background())
	rt := &Runtime{
		root:          root,
		store:         store,
		startedAt:     time.Now().UTC(),
		deviceName:    defaultDeviceName(opts.DeviceName),
		platform:      defaultPlatform(opts.Platform),
		appVersion:    strings.TrimSpace(opts.AppVersion),
		ctx:           ctx,
		cancel:        cancel,
		meta:          meta,
		pingGate:      make(map[string]int64),
		shellSessions: make(map[string]*shellSessionState),
		subscribers:   make(map[int]chan Event),
		peerSessions:  dataplane.NewSessionManager(),
	}
	if rt.appVersion == "" {
		rt.appVersion = buildinfo.Version()
	}
	if strings.TrimSpace(opts.BrokerURL) != "" {
		rt.meta.RuntimeBrokerOverride = normalizeBrokerEndpoint(opts.BrokerURL)
	}
	rt.peerSessions.SetChangeHook(func() {
		rt.notifyChange("snapshot.updated")
	})

	if strings.TrimSpace(meta.ActiveNetworkID) != "" {
		if err := rt.ensureWorkers(ctx); err != nil {
			rt.setStatus(wrapProblem(
				StageEnroll,
				poc.ReasonCodeUnavailable,
				poc.ExitCodeUnavailable,
				"failed to restore joined runtime workers",
				err,
				"retry",
			))
		}
	}
	return rt, nil
}

func (r *Runtime) Close() error {
	if r == nil {
		return nil
	}

	r.mu.Lock()
	if r.closed {
		r.mu.Unlock()
		return nil
	}
	r.closed = true
	r.cancel()
	subscribers := r.subscribers
	r.subscribers = map[int]chan Event{}
	shells := make([]*shellSessionState, 0, len(r.shellSessions))
	for _, sessionState := range r.shellSessions {
		shells = append(shells, sessionState)
	}
	r.shellSessions = map[string]*shellSessionState{}
	presenceSession := r.presence
	r.presence = nil
	udpConn := r.udpConn
	r.udpConn = nil
	broker := r.broker
	r.broker = nil
	r.mu.Unlock()

	for _, ch := range subscribers {
		close(ch)
	}
	for _, sessionState := range shells {
		if sessionState != nil && sessionState.stream != nil {
			_ = sessionState.stream.Close()
		}
	}
	r.peerSessions.CloseAll(dataplane.CloseReasonDaemonShutdown)
	if presenceSession != nil {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		_ = presenceSession.Close(ctx)
		cancel()
	}
	if udpConn != nil {
		_ = udpConn.Close()
	}
	if broker != nil {
		_ = broker.Close()
	}
	r.wg.Wait()
	return nil
}

func (r *Runtime) Status() StatusResponse {
	if r == nil {
		return StatusResponse{}
	}
	now := time.Now().UTC()
	return StatusResponse{
		Version:   buildinfo.Version(),
		StartedAt: r.startedAt,
		UptimeMs:  now.Sub(r.startedAt).Milliseconds(),
	}
}

func (r *Runtime) Snapshot() Snapshot {
	if r == nil {
		return Snapshot{
			Stage:      StageNetwork,
			ReasonCode: poc.ReasonCodeInternal,
			Summary:    UserSummary{Text: "runtime unavailable"},
			Evidence:   Evidence{},
		}
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.snapshotLocked()
}

func (r *Runtime) Subscribe() (int, <-chan Event) {
	ch := make(chan Event, 16)
	r.mu.Lock()
	r.nextSubscriberID++
	id := r.nextSubscriberID
	r.subscribers[id] = ch
	snapshot := r.snapshotLocked()
	r.mu.Unlock()

	ch <- Event{
		Kind:     "snapshot",
		AtUnixMs: time.Now().UTC().UnixMilli(),
		Snapshot: snapshot,
	}
	return id, ch
}

func (r *Runtime) Unsubscribe(id int) {
	r.mu.Lock()
	ch, ok := r.subscribers[id]
	if ok {
		delete(r.subscribers, id)
	}
	r.mu.Unlock()
	if ok {
		close(ch)
	}
}

func (r *Runtime) AttachShell(ctx context.Context, shellSessionID string, conn io.ReadWriteCloser) *problem {
	if ctx == nil {
		ctx = context.Background()
	}
	if conn == nil {
		return newProblem(
			StageShell,
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"shell stream connection is required",
			nil,
			[]poc.Suggestion{{Message: "retry"}},
		)
	}

	r.mu.Lock()
	state, ok := r.shellSessions[strings.TrimSpace(shellSessionID)]
	if !ok || state == nil {
		r.mu.Unlock()
		return newProblem(
			StageShell,
			poc.ReasonCodeNotFound,
			poc.ExitCodeNotFound,
			"shell session not found",
			[]poc.Fact{{Message: "shell_session_id=" + strings.TrimSpace(shellSessionID)}},
			[]poc.Suggestion{{Message: "run: miopunch sh <peer_id>"}},
		)
	}
	if state.attached {
		r.mu.Unlock()
		return newProblem(
			StageShell,
			poc.ReasonCodeConflict,
			poc.ExitCodeConflict,
			"shell session is already attached",
			[]poc.Fact{{Message: "shell_session_id=" + strings.TrimSpace(shellSessionID)}},
			[]poc.Suggestion{{Message: "close the existing shell and retry"}},
		)
	}
	state.attached = true
	state.summary.Status = "attached"
	state.summary.AttachedUnixMs = time.Now().UTC().UnixMilli()
	stream := state.stream
	r.mu.Unlock()
	r.notifyChange("snapshot.updated")

	err := proxyShell(ctx, conn, stream)

	r.mu.Lock()
	if current, ok := r.shellSessions[strings.TrimSpace(shellSessionID)]; ok && current == state {
		state.summary.Status = "closed"
		state.summary.ClosedAtUnixMs = time.Now().UTC().UnixMilli()
		delete(r.shellSessions, strings.TrimSpace(shellSessionID))
	}
	r.mu.Unlock()
	r.notifyChange("snapshot.updated")

	if err == nil || errors.Is(err, io.EOF) || errors.Is(err, net.ErrClosed) {
		return nil
	}
	return wrapProblem(StageShell, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "shell stream closed unexpectedly", err, "retry")
}

func (r *Runtime) snapshotLocked() Snapshot {
	stage := r.derivedStageLocked()
	reasonCode := poc.ReasonCodeOK
	summaryText := r.summaryTextLocked(stage)
	evidence := Evidence{Facts: []poc.Fact{}, Suggestions: []poc.Suggestion{}}
	if r.status != nil {
		stage = r.status.stage
		reasonCode = r.status.reasonCode
		if strings.TrimSpace(r.status.message) != "" {
			summaryText = r.status.message
		}
		evidence = Evidence{
			Facts:       cloneFacts(r.status.facts),
			Suggestions: cloneSuggestions(r.status.suggestions),
		}
	}

	peerSessions := r.peerSessionSummariesLocked()
	shellSessions := r.shellSessionSummariesLocked()
	discover := r.discoverProjectionLocked()

	if len(evidence.Facts) == 0 {
		evidence.Facts = append(evidence.Facts, r.contextFactsLocked(discover, peerSessions, shellSessions)...)
	}
	return Snapshot{
		Stage:         stage,
		ReasonCode:    reasonCode,
		Summary:       UserSummary{Text: summaryText},
		Evidence:      evidence,
		DiscoverView:  discover,
		PeerSessions:  peerSessions,
		ShellSessions: shellSessions,
	}
}

func (r *Runtime) contextFactsLocked(discover presence.DiscoverProjection, peers []PeerSession, shells []ShellSession) []poc.Fact {
	facts := []poc.Fact{}
	if strings.TrimSpace(r.meta.ActiveNetworkID) != "" {
		facts = append(facts, poc.Fact{Message: "network_id=" + r.meta.ActiveNetworkID})
	}
	if strings.TrimSpace(r.meta.Role) != "" {
		facts = append(facts, poc.Fact{Message: "role=" + r.meta.Role})
	}
	if strings.TrimSpace(r.meta.BrokerEndpoint) != "" {
		facts = append(facts, poc.Fact{Message: "broker_endpoint=" + r.meta.BrokerEndpoint})
	}
	if len(discover.Peers) > 0 {
		facts = append(facts, poc.Fact{Message: fmt.Sprintf("discover_peer_count=%d", len(discover.Peers))})
	}
	if len(peers) > 0 {
		facts = append(facts, poc.Fact{Message: fmt.Sprintf("peer_session_count=%d", len(peers))})
	}
	if len(shells) > 0 {
		facts = append(facts, poc.Fact{Message: fmt.Sprintf("shell_session_count=%d", len(shells))})
	}
	return facts
}

func (r *Runtime) summaryTextLocked(stage Stage) string {
	switch stage {
	case StageNetwork:
		if strings.TrimSpace(r.meta.ActiveNetworkID) == "" {
			return "network is not initialized"
		}
		return "network bootstrap is ready"
	case StageEnroll:
		return "joined network is restoring enrollment state"
	case StageDiscover:
		return "discover session is online"
	case StagePunch:
		return "punching is in progress"
	case StageSecureSession:
		return "secure peer session is ready"
	case StageShell:
		return "shell gate is satisfied"
	default:
		return "runtime is ready"
	}
}

func (r *Runtime) derivedStageLocked() Stage {
	if strings.TrimSpace(r.meta.ActiveNetworkID) == "" {
		return StageNetwork
	}
	if r.presence == nil {
		return StageEnroll
	}
	if len(r.shellSessions) > 0 {
		return StageShell
	}
	for _, unixMs := range r.pingGate {
		if unixMs > 0 {
			return StageShell
		}
	}
	if len(r.peerSessions.ListSummaries()) > 0 {
		return StageSecureSession
	}
	return StageDiscover
}

func (r *Runtime) discoverProjectionLocked() presence.DiscoverProjection {
	if r.presence != nil {
		return presence.ProjectView(r.presence.View())
	}
	if strings.TrimSpace(r.meta.ActiveNetworkID) == "" {
		return presence.DiscoverProjection{}
	}
	roster, err := r.store.LoadRosterSnapshot(r.meta.ActiveNetworkID)
	if err != nil {
		return presence.DiscoverProjection{
			NetworkID:  r.meta.ActiveNetworkID,
			SelfPeerID: r.selfPeerID(),
		}
	}
	peers := make([]presence.DiscoverProjectionPeer, 0, len(roster.Entries))
	selfPeerID := r.selfPeerID()
	for _, entry := range roster.Entries {
		if entry.PeerID == selfPeerID {
			continue
		}
		peers = append(peers, presence.DiscoverProjectionPeer{
			PeerID:      entry.PeerID,
			OnlineState: presence.OnlineStateOffline,
			DeviceName:  entry.DeviceName,
			Platform:    entry.Platform,
		})
	}
	return presence.DiscoverProjection{
		NetworkID:  r.meta.ActiveNetworkID,
		SelfPeerID: selfPeerID,
		Peers:      peers,
	}
}

func (r *Runtime) peerSessionSummariesLocked() []PeerSession {
	source := r.peerSessions.ListAllSummaries()
	out := make([]PeerSession, 0, len(source))
	for _, summary := range source {
		provenAt := r.pingGate[summary.Key.RemotePeerID]
		out = append(out, PeerSession{
			PeerID:             summary.Key.RemotePeerID,
			Healthy:            summary.Healthy,
			PathFamily:         string(summary.Key.PathFamily),
			Protocol:           string(summary.Key.Protocol),
			LocalEndpoint:      summary.PathFacts.LocalEndpoint,
			RemoteEndpoint:     summary.PathFacts.RemoteEndpoint,
			LastActivityUnixMs: summary.LastActivityUnixMilli,
			PingGateSatisfied:  provenAt > 0,
			ShellReadyUnixMs:   provenAt,
			LastProvenUnixMs:   provenAt,
		})
	}
	sort.Slice(out, func(i, j int) bool {
		return out[i].PeerID < out[j].PeerID
	})
	return out
}

func (r *Runtime) shellSessionSummariesLocked() []ShellSession {
	out := make([]ShellSession, 0, len(r.shellSessions))
	for _, state := range r.shellSessions {
		if state == nil {
			continue
		}
		out = append(out, state.summary)
	}
	sort.Slice(out, func(i, j int) bool {
		return out[i].ID < out[j].ID
	})
	return out
}

func (r *Runtime) setStatus(problem *problem) {
	r.mu.Lock()
	r.status = problem
	r.mu.Unlock()
	r.notifyChange("snapshot.updated")
}

func (r *Runtime) clearStatus() {
	r.mu.Lock()
	r.status = nil
	r.mu.Unlock()
	r.notifyChange("snapshot.updated")
}

func (r *Runtime) notifyChange(kind string) {
	r.mu.Lock()
	snapshot := r.snapshotLocked()
	subscribers := make([]chan Event, 0, len(r.subscribers))
	for _, ch := range r.subscribers {
		subscribers = append(subscribers, ch)
	}
	r.mu.Unlock()

	event := Event{
		Kind:     kind,
		AtUnixMs: time.Now().UTC().UnixMilli(),
		Snapshot: snapshot,
	}
	for _, ch := range subscribers {
		select {
		case ch <- event:
		default:
		}
	}
}

func (r *Runtime) ensureWorkers(ctx context.Context) error {
	if ctx == nil {
		ctx = context.Background()
	}

	r.mu.Lock()
	networkID := strings.TrimSpace(r.meta.ActiveNetworkID)
	role := strings.TrimSpace(r.meta.Role)
	brokerEndpoint := strings.TrimSpace(r.meta.BrokerEndpoint)
	brokerOverride := strings.TrimSpace(r.meta.RuntimeBrokerOverride)
	presenceSession := r.presence
	udpConn := r.udpConn
	acceptLoopStarted := r.acceptLoopStarted
	keepaliveStarted := r.keepaliveStarted
	r.mu.Unlock()

	if networkID == "" {
		return nil
	}

	if role == "admin" && brokerOverride == "" {
		r.mu.Lock()
		currentBroker := r.broker
		r.mu.Unlock()
		if currentBroker == nil {
			broker, err := startEmbeddedBroker(brokerEndpoint)
			if err != nil {
				return fmt.Errorf("start embedded broker: %w", err)
			}
			r.mu.Lock()
			if r.broker == nil {
				r.broker = broker
				if strings.TrimSpace(r.meta.BrokerEndpoint) == "" || strings.TrimSpace(r.meta.BrokerEndpoint) != broker.Endpoint() {
					r.meta.BrokerEndpoint = broker.Endpoint()
					_ = saveMetadata(r.root, r.meta)
				}
			} else {
				_ = broker.Close()
			}
			r.mu.Unlock()
		}
	}

	if presenceSession == nil {
		cfg, err := presence.LoadConfig(r.store, networkID, r.deviceName, r.platform, r.appVersion)
		if err != nil {
			return fmt.Errorf("load presence config: %w", err)
		}
		sess, err := presence.OpenSession(ctx, cfg)
		if err != nil {
			return fmt.Errorf("open presence session: %w", err)
		}
		r.mu.Lock()
		if r.presence == nil {
			r.presence = sess
		} else {
			_ = sess.Close(context.Background())
		}
		r.mu.Unlock()
	}

	if udpConn == nil {
		conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4zero, Port: 0})
		if err != nil {
			return fmt.Errorf("listen udp: %w", err)
		}
		r.mu.Lock()
		if r.udpConn == nil {
			r.udpConn = conn
		} else {
			_ = conn.Close()
		}
		r.mu.Unlock()
	}

	if !acceptLoopStarted {
		r.mu.Lock()
		if !r.acceptLoopStarted {
			r.acceptLoopStarted = true
			r.wg.Add(1)
			go r.acceptLoop()
		}
		r.mu.Unlock()
	}
	if !keepaliveStarted {
		r.mu.Lock()
		if !r.keepaliveStarted {
			r.keepaliveStarted = true
			r.wg.Add(1)
			go r.keepaliveLoop()
		}
		r.mu.Unlock()
	}
	return nil
}

func (r *Runtime) refreshPresenceRoster(ctx context.Context) error {
	if ctx == nil {
		ctx = context.Background()
	}

	r.mu.Lock()
	networkID := strings.TrimSpace(r.meta.ActiveNetworkID)
	presenceSession := r.presence
	r.mu.Unlock()
	if networkID == "" || presenceSession == nil {
		return nil
	}

	cfg, err := presence.LoadConfig(r.store, networkID, r.deviceName, r.platform, r.appVersion)
	if err != nil {
		return fmt.Errorf("load presence config: %w", err)
	}
	if err := presenceSession.ReloadRosterSnapshot(cfg.RosterSnapshot); err != nil {
		return fmt.Errorf("reload presence roster snapshot: %w", err)
	}
	return nil
}

func (r *Runtime) acceptLoop() {
	defer r.wg.Done()
	for {
		if err := r.ctx.Err(); err != nil {
			return
		}

		cfg, problem := r.punchConfig()
		if problem != nil {
			r.setStatus(problem)
			select {
			case <-time.After(500 * time.Millisecond):
			case <-r.ctx.Done():
				return
			}
			continue
		}
		result, err := punch.HandleOne(r.ctx, cfg)
		if err != nil {
			if r.ctx.Err() != nil {
				return
			}
			var diagnosticErr *punch.Error
			if errors.As(err, &diagnosticErr) {
				logutil.Warnf(
					"punch accept failed: remote_peer_id=%s dial_id=%s planned_pairs=%d attempt_results=%s err=%v",
					diagnosticErr.Diagnostic.RemotePeerID,
					diagnosticErr.Diagnostic.DialID,
					diagnosticErr.Diagnostic.PlannedPairCount,
					summarizePunchFacts(diagnosticErr.Diagnostic),
					err,
				)
			} else {
				logutil.Warnf("punch accept failed: err=%v", err)
			}
			continue
		}

		sess, err := session.Accept(r.ctx, r.sessionConfig(), result)
		if err != nil {
			_ = result.Close()
			if r.ctx.Err() != nil {
				return
			}
			continue
		}
		logutil.Infof(
			"punch accept selected: remote_peer_id=%s remote_udp=%s",
			result.RemoteIdentity.PeerID,
			result.Evidence.SelectedRemoteUDP,
		)
		r.registerPeerSession(result.RemoteIdentity.PeerID, sess)
	}
}

func (r *Runtime) registerPeerSession(peerID string, sess session.PeerSession) {
	if sess == nil {
		return
	}
	r.peerSessions.Put(sess)

	r.mu.Lock()
	if _, ok := r.pingGate[peerID]; !ok {
		r.pingGate[peerID] = 0
	}
	r.mu.Unlock()
	r.notifyChange("snapshot.updated")

	r.wg.Add(1)
	go func() {
		defer r.wg.Done()
		r.servePeerSession(peerID, sess)
	}()
}

func (r *Runtime) keepaliveLoop() {
	defer r.wg.Done()

	ticker := time.NewTicker(sessionKeepaliveInterval)
	defer ticker.Stop()

	for {
		select {
		case <-r.ctx.Done():
			return
		case <-ticker.C:
			r.keepaliveValidatedSessions()
		}
	}
}

func (r *Runtime) keepaliveValidatedSessions() {
	now := time.Now().UTC()
	for _, summary := range r.peerSessions.ListSummaries() {
		key := summary.Key.Normalize()
		if key.RemotePeerID == "" || !summary.Healthy {
			continue
		}
		if !r.hasPingGate(key.RemotePeerID) {
			continue
		}

		lastActivity := time.UnixMilli(summary.LastActivityUnixMilli).UTC()
		if !lastActivity.IsZero() && now.Sub(lastActivity) < sessionKeepaliveMinIdle {
			continue
		}

		sess, ok := r.peerSessions.Get(key)
		if !ok {
			continue
		}
		if err := r.keepaliveSession(sess, key.RemotePeerID); err != nil {
			logutil.Warnf(
				"runtime session keepalive failed: peer_id=%s proto=%s path_family=%s err=%v",
				key.RemotePeerID,
				key.Protocol,
				key.PathFamily,
				err,
			)
			r.peerSessions.Close(key, dataplane.CloseReasonTransportFatal)
		}
	}
}

func (r *Runtime) keepaliveSession(sess session.PeerSession, peerID string) error {
	if sess == nil {
		return errors.New("nil peer session")
	}

	ctx, cancel := context.WithTimeout(r.ctx, sessionKeepaliveTimeout)
	defer cancel()

	if problem := r.exchangePing(ctx, sess, peerID); problem != nil {
		return problem
	}
	return nil
}

func (r *Runtime) servePeerSession(peerID string, sess session.PeerSession) {
	defer r.notifyChange("snapshot.updated")

	for {
		if err := r.ctx.Err(); err != nil {
			return
		}
		accepted, err := session.AcceptStream(r.ctx, sess)
		if err != nil {
			if r.ctx.Err() != nil {
				return
			}
			return
		}
		r.wg.Add(1)
		go func() {
			defer r.wg.Done()
			r.handleAcceptedStream(peerID, accepted)
		}()
	}
}

func (r *Runtime) handleAcceptedStream(peerID string, accepted *session.AcceptedStream) {
	if accepted == nil || accepted.Stream == nil {
		return
	}
	defer func() { _ = accepted.Stream.Close() }()
	if accepted.Open.Kind != dataplane.StreamKindShellV0 {
		return
	}

	var control shellproto.Control
	if err := shellproto.ReadJSON(accepted.Stream, &control); err != nil {
		return
	}

	switch control.Op {
	case shellproto.OpHello, shellproto.OpPing:
		r.markPingGate(peerID)
		_ = shellproto.WriteJSON(accepted.Stream, shellproto.Control{
			Op: control.Op,
			OK: true,
		})
	case shellproto.OpShLS:
		r.handleRemoteShellList(accepted.Stream, control)
	case shellproto.OpShAttach:
		r.handleRemoteShellAttach(peerID, accepted.Stream, control)
	default:
		_ = shellproto.WriteJSON(accepted.Stream, shellproto.Control{
			Op: control.Op,
			Error: &shellproto.ControlError{
				ReasonCode: string(poc.ReasonCodeBadRequest),
				Message:    "unsupported shell operation",
			},
		})
	}
}

func (r *Runtime) handleRemoteShellList(stream io.ReadWriteCloser, control shellproto.Control) {
	ctx, cancel := context.WithTimeout(r.ctx, 30*time.Second)
	defer cancel()

	targets, err := shelltarget.ListTargets(ctx)
	if err != nil {
		_ = shellproto.WriteJSON(stream, shellproto.Control{
			Op: shellproto.OpShLS,
			Error: &shellproto.ControlError{
				ReasonCode: string(poc.ReasonCodeUnavailable),
				Message:    err.Error(),
			},
		})
		return
	}

	resolved := strings.TrimSpace(control.Target)
	if control.ReadyOnly && resolved != "" {
		_ = shellproto.WriteJSON(stream, shellproto.Control{
			Op: shellproto.OpShLS,
			Error: &shellproto.ControlError{
				ReasonCode: string(poc.ReasonCodeBadRequest),
				Message:    "--ready cannot be combined with a concrete target",
				Suggestions: []string{
					"use: miopunch sh ls <peer_id> --ready",
					"or: miopunch sh ls <peer_id> <target>",
				},
			},
		})
		return
	}
	if resolved != "" {
		resolved, err = shelltarget.Resolve(control.Target, targets)
		if err != nil {
			_ = shellproto.WriteJSON(stream, shellproto.Control{
				Op:    shellproto.OpShLS,
				Error: shellControlError(err),
			})
			return
		}
	}

	sessions := []string{}
	targetStatuses := []shellproto.TargetStatus{}
	if resolved != "" {
		sessions, err = shelltarget.ListSessions(ctx, resolved)
		if err != nil {
			_ = shellproto.WriteJSON(stream, shellproto.Control{
				Op:    shellproto.OpShLS,
				Error: shellControlError(err),
			})
			return
		}
	} else if control.ReadyOnly {
		targets, targetStatuses = probeReadyTargets(ctx, targets, shelltarget.ProbeReadiness)
	}

	_ = shellproto.WriteJSON(stream, shellproto.Control{
		Op:             shellproto.OpShLS,
		OK:             true,
		Targets:        targets,
		Sessions:       sessions,
		Target:         resolved,
		ReadyOnly:      control.ReadyOnly,
		TargetStatuses: targetStatuses,
	})
}

type shellTargetReadinessProbe func(context.Context, string) (shelltarget.TargetReadiness, error)

func probeReadyTargets(ctx context.Context, targets []string, probe shellTargetReadinessProbe) ([]string, []shellproto.TargetStatus) {
	filteredTargets := make([]string, 0, len(targets))
	for _, target := range targets {
		target = strings.TrimSpace(target)
		if target == "" {
			continue
		}
		filteredTargets = append(filteredTargets, target)
	}
	if len(filteredTargets) == 0 {
		return []string{}, []shellproto.TargetStatus{}
	}

	type probeOutcome struct {
		status shellproto.TargetStatus
		ready  bool
	}

	outcomes := make([]probeOutcome, len(filteredTargets))
	var wg sync.WaitGroup
	wg.Add(len(filteredTargets))
	for idx, target := range filteredTargets {
		idx, target := idx, target
		go func() {
			defer wg.Done()

			readiness, err := probe(ctx, target)
			if err != nil {
				outcomes[idx].status = readinessStatusFromError(target, err)
				return
			}

			status := shellproto.TargetStatus{
				Target:     target,
				Status:     readiness.Status,
				ReasonCode: readiness.ReasonCode,
				Message:    readiness.Message,
			}
			if strings.TrimSpace(status.Target) == "" {
				status.Target = target
			}
			outcomes[idx].status = status
			outcomes[idx].ready = status.Status == shelltarget.TargetStatusReady
		}()
	}
	wg.Wait()

	readyTargets := make([]string, 0, len(filteredTargets))
	targetStatuses := make([]shellproto.TargetStatus, 0, len(filteredTargets))
	for idx, target := range filteredTargets {
		status := outcomes[idx].status
		if strings.TrimSpace(status.Target) == "" {
			status.Target = target
		}
		targetStatuses = append(targetStatuses, status)
		if outcomes[idx].ready {
			readyTargets = append(readyTargets, target)
		}
	}
	return readyTargets, targetStatuses
}

func readinessStatusFromError(target string, err error) shellproto.TargetStatus {
	status := shellproto.TargetStatus{
		Target:     target,
		Status:     shelltarget.TargetStatusUnknown,
		ReasonCode: string(poc.ReasonCodeUnavailable),
		Message:    strings.TrimSpace(err.Error()),
	}
	if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
		status.ReasonCode = string(poc.ReasonCodeTimeout)
		status.Message = context.DeadlineExceeded.Error()
	}
	return status
}

func (r *Runtime) handleRemoteShellAttach(peerID string, stream io.ReadWriteCloser, control shellproto.Control) {
	if !r.hasPingGate(peerID) {
		_ = shellproto.WriteJSON(stream, shellproto.Control{
			Op: shellproto.OpShAttach,
			Error: &shellproto.ControlError{
				ReasonCode: shellproto.ReasonHelloRequired,
				Message:    "shell attach requires a successful ping or hello first",
				Suggestions: []string{
					"run: miopunch ping <peer_id>",
				},
			},
		})
		return
	}

	ctx, cancel := context.WithTimeout(r.ctx, 15*time.Second)
	defer cancel()

	targets, err := shelltarget.ListTargets(ctx)
	if err != nil {
		_ = shellproto.WriteJSON(stream, shellproto.Control{
			Op:    shellproto.OpShAttach,
			Error: shellControlError(err),
		})
		return
	}
	target, err := shelltarget.Resolve(control.Target, targets)
	if err != nil {
		_ = shellproto.WriteJSON(stream, shellproto.Control{
			Op:    shellproto.OpShAttach,
			Error: shellControlError(err),
		})
		return
	}
	pty, err := shelltarget.Attach(r.ctx, target, control.Session)
	if err != nil {
		_ = shellproto.WriteJSON(stream, shellproto.Control{
			Op:    shellproto.OpShAttach,
			Error: shellControlError(err),
		})
		return
	}
	defer func() { _ = pty.Close() }()

	sessionID, _ := wire.NewMsgID()
	cleanup := r.addTransientShellSession(sessionID, peerID, target, defaultShellSessionName(control.Session))
	defer cleanup()

	if err := shellproto.WriteJSON(stream, shellproto.Control{
		Op:      shellproto.OpShAttach,
		OK:      true,
		Target:  target,
		Session: defaultShellSessionName(control.Session),
	}); err != nil {
		return
	}

	_ = bridgeRemotePTY(r.ctx, stream, pty)
}

func (r *Runtime) addTransientShellSession(id, peerID, target, sessionName string) func() {
	nowUnixMs := time.Now().UTC().UnixMilli()
	r.mu.Lock()
	r.shellSessions[id] = &shellSessionState{
		summary: ShellSession{
			ID:              id,
			PeerID:          peerID,
			Target:          target,
			Session:         sessionName,
			Status:          "remote",
			CreatedAtUnixMs: nowUnixMs,
			AttachedUnixMs:  nowUnixMs,
		},
		attached: true,
	}
	r.mu.Unlock()
	r.notifyChange("snapshot.updated")

	return func() {
		r.mu.Lock()
		if current, ok := r.shellSessions[id]; ok && current != nil {
			current.summary.Status = "closed"
			current.summary.ClosedAtUnixMs = time.Now().UTC().UnixMilli()
			delete(r.shellSessions, id)
		}
		r.mu.Unlock()
		r.notifyChange("snapshot.updated")
	}
}

func (r *Runtime) selfPeerID() string {
	deviceKeys, err := r.store.LoadDeviceKeys()
	if err != nil {
		return ""
	}
	peerID, err := deviceKeys.PeerID()
	if err != nil {
		return ""
	}
	return peerID
}

func (r *Runtime) authorityEd25519() (ed25519.PublicKey, error) {
	r.mu.Lock()
	meta := r.meta
	r.mu.Unlock()
	return meta.authorityEd25519()
}

func (r *Runtime) authorityX25519() ([]byte, error) {
	r.mu.Lock()
	meta := r.meta
	r.mu.Unlock()
	return meta.authorityX25519()
}

func (r *Runtime) sessionConfig() session.Config {
	pub, _ := r.authorityEd25519()
	return session.Config{
		NetworkID:           strings.TrimSpace(r.meta.ActiveNetworkID),
		AuthorityEd25519Pub: pub,
		Store:               r.store,
		IdleTimeout:         defaultIdleTimeout,
	}
}

func (r *Runtime) punchConfig() (punch.Config, *problem) {
	pub, err := r.authorityEd25519()
	if err != nil || len(pub) == 0 {
		return punch.Config{}, wrapProblem(StagePunch, poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, "authority key is unavailable", err, "re-run init-network or join")
	}

	r.mu.Lock()
	networkID := strings.TrimSpace(r.meta.ActiveNetworkID)
	presenceSession := r.presence
	udpConn := r.udpConn
	r.mu.Unlock()
	if networkID == "" {
		return punch.Config{}, newProblem(
			StageNetwork,
			poc.ReasonCodeNotFound,
			poc.ExitCodeNotFound,
			"no active network is joined",
			nil,
			[]poc.Suggestion{{Message: "run: miopunch init-network or miopunch join <invite_code>"}},
		)
	}
	if udpConn == nil {
		return punch.Config{}, newProblem(
			StagePunch,
			poc.ReasonCodeUnavailable,
			poc.ExitCodeUnavailable,
			"udp runtime is unavailable",
			nil,
			[]poc.Suggestion{{Message: "retry"}},
		)
	}

	discover := presence.DiscoverProjection{}
	if presenceSession != nil {
		discover = presence.ProjectView(presenceSession.View())
	}
	return punch.Config{
		NetworkID:           networkID,
		AuthorityEd25519Pub: pub,
		Store:               r.store,
		Discover:            discover,
		LocalCandidates:     localCandidates(udpConn),
		UDPConn:             udpConn,
	}, nil
}

func (r *Runtime) markPingGate(peerID string) {
	r.mu.Lock()
	r.pingGate[strings.TrimSpace(peerID)] = time.Now().UTC().UnixMilli()
	r.mu.Unlock()
	r.notifyChange("snapshot.updated")
}

func (r *Runtime) hasPingGate(peerID string) bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.pingGate[strings.TrimSpace(peerID)] > 0
}

func localCandidates(conn *net.UDPConn) []punch.Candidate {
	if conn == nil {
		return nil
	}
	localAddr, ok := conn.LocalAddr().(*net.UDPAddr)
	if !ok || localAddr == nil {
		return nil
	}
	port := localAddr.Port
	if port <= 0 {
		return nil
	}

	ifaceAddrs := collectLocalInterfaceAddrs()
	candidates := localCandidatesForPort(port, ifaceAddrs)
	logutil.Debugf(
		"punch local candidate gather complete: port=%d candidate_count=%d candidates=%s",
		port,
		len(candidates),
		formatLocalCandidates(candidates),
	)
	return candidates
}

type localInterfaceAddr struct {
	Name  string
	Flags net.Flags
	Addr  net.Addr
}

func collectLocalInterfaceAddrs() []localInterfaceAddr {
	interfaces, err := net.Interfaces()
	if err != nil {
		logutil.Debugf("punch local candidate gather fallback: list interfaces err=%v", err)
		return collectUnnamedInterfaceAddrs()
	}
	out := make([]localInterfaceAddr, 0, len(interfaces))
	for _, iface := range interfaces {
		addrs, err := iface.Addrs()
		if err != nil {
			logutil.Debugf(
				"punch local candidate gather skip: iface=%q flags=%s reason=addr_list_failed err=%v",
				iface.Name,
				iface.Flags.String(),
				err,
			)
			continue
		}
		for _, addr := range addrs {
			out = append(out, localInterfaceAddr{
				Name:  iface.Name,
				Flags: iface.Flags,
				Addr:  addr,
			})
		}
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].Name == out[j].Name {
			return out[i].Addr.String() < out[j].Addr.String()
		}
		return out[i].Name < out[j].Name
	})
	return out
}

func collectUnnamedInterfaceAddrs() []localInterfaceAddr {
	addrs, err := net.InterfaceAddrs()
	if err != nil {
		logutil.Debugf("punch local candidate gather failed: list interface addrs err=%v", err)
		return nil
	}
	out := make([]localInterfaceAddr, 0, len(addrs))
	for _, addr := range addrs {
		out = append(out, localInterfaceAddr{Addr: addr})
	}
	return out
}

func localCandidatesForPort(port int, ifaceAddrs []localInterfaceAddr) []punch.Candidate {
	if port <= 0 {
		return nil
	}

	seen := map[string]struct{}{}
	out := []punch.Candidate{}
	appendAddr := func(host string, source localInterfaceAddr) {
		host = strings.TrimSpace(host)
		if host == "" {
			return
		}
		addr := net.JoinHostPort(host, fmt.Sprintf("%d", port))
		if _, ok := seen[addr]; ok {
			logutil.Debugf(
				"punch local candidate skip: iface=%q flags=%s candidate=%s reason=duplicate",
				source.Name,
				source.Flags.String(),
				addr,
			)
			return
		}
		seen[addr] = struct{}{}
		logutil.Debugf(
			"punch local candidate accept: iface=%q flags=%s candidate=%s reason=usable_ipv4",
			source.Name,
			source.Flags.String(),
			addr,
		)
		out = append(out, punch.Candidate{Kind: punch.CandidateKindHost, Addr: addr})
	}

	for _, source := range ifaceAddrs {
		ip := localIPv4(source.Addr)
		if ip == nil {
			logutil.Debugf(
				"punch local candidate skip: iface=%q flags=%s raw=%q reason=non_ipv4",
				source.Name,
				source.Flags.String(),
				addrString(source.Addr),
			)
			continue
		}
		ok, reason := allowLocalCandidate(source, ip)
		if !ok {
			logutil.Debugf(
				"punch local candidate skip: iface=%q flags=%s ip=%s reason=%s",
				source.Name,
				source.Flags.String(),
				ip.String(),
				reason,
			)
			continue
		}
		appendAddr(ip.String(), source)
	}
	if len(out) == 0 {
		logutil.Debugf(
			"punch local candidate fallback: port=%d candidate=%s reason=no_usable_ipv4",
			port,
			net.JoinHostPort("127.0.0.1", fmt.Sprintf("%d", port)),
		)
		appendAddr("127.0.0.1", localInterfaceAddr{Name: "loopback-fallback", Flags: net.FlagLoopback})
	}
	return out
}

func localIPv4(addr net.Addr) net.IP {
	switch value := addr.(type) {
	case *net.IPNet:
		if value == nil || value.IP == nil {
			return nil
		}
		return value.IP.To4()
	case *net.IPAddr:
		if value == nil || value.IP == nil {
			return nil
		}
		return value.IP.To4()
	default:
		return nil
	}
}

func allowLocalCandidate(source localInterfaceAddr, ip net.IP) (bool, string) {
	if ip == nil {
		return false, "missing_ip"
	}
	if source.Flags&net.FlagUp == 0 && source.Flags != 0 {
		return false, "iface_down"
	}
	if source.Flags&net.FlagLoopback != 0 {
		return false, "iface_loopback"
	}
	if ip.IsLoopback() {
		return false, "loopback_ip"
	}
	if ip.IsLinkLocalUnicast() || ip.IsLinkLocalMulticast() {
		return false, "link_local_ip"
	}
	if isRejectedLocalInterface(source.Name) {
		return false, "virtual_iface"
	}
	return true, "usable_ipv4"
}

func isRejectedLocalInterface(name string) bool {
	lowerName := strings.ToLower(strings.TrimSpace(name))
	if lowerName == "" {
		return false
	}
	switch {
	case strings.HasPrefix(lowerName, "docker"):
		return true
	case strings.HasPrefix(lowerName, "br-"):
		return true
	case strings.HasPrefix(lowerName, "veth"):
		return true
	case strings.HasPrefix(lowerName, "virbr"):
		return true
	case strings.HasPrefix(lowerName, "cni"):
		return true
	case strings.Contains(lowerName, "default switch"):
		return true
	default:
		return false
	}
}

func formatLocalCandidates(candidates []punch.Candidate) string {
	if len(candidates) == 0 {
		return "none"
	}
	parts := make([]string, 0, len(candidates))
	for _, candidate := range candidates {
		parts = append(parts, string(candidate.Kind)+"@"+candidate.Addr)
	}
	return strings.Join(parts, ",")
}

func addrString(addr net.Addr) string {
	if addr == nil {
		return ""
	}
	return addr.String()
}

func summarizePunchFacts(diag punch.Diagnostic) string {
	facts := diag.Facts()
	parts := make([]string, 0, len(facts))
	for _, fact := range facts {
		if strings.TrimSpace(fact.Message) == "" {
			continue
		}
		parts = append(parts, fact.Message)
	}
	return strings.Join(parts, " ")
}

func proxyShell(ctx context.Context, local io.ReadWriteCloser, remote io.ReadWriteCloser) error {
	if ctx == nil {
		ctx = context.Background()
	}
	if local == nil || remote == nil {
		return errors.New("shell streams are required")
	}

	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	errCh := make(chan error, 2)
	pipe := func(dst io.WriteCloser, src io.ReadCloser) {
		_, err := io.Copy(dst, src)
		errCh <- err
	}

	go pipe(remote, local)
	go pipe(local, remote)

	select {
	case <-ctx.Done():
		_ = local.Close()
		_ = remote.Close()
		return ctx.Err()
	case err := <-errCh:
		_ = local.Close()
		_ = remote.Close()
		if err == nil || errors.Is(err, io.EOF) || errors.Is(err, net.ErrClosed) {
			return nil
		}
		return err
	}
}

func bridgeRemotePTY(ctx context.Context, stream io.ReadWriteCloser, pty shelltarget.PTY) error {
	if ctx == nil {
		ctx = context.Background()
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	errCh := make(chan error, 3)
	closeAll := func() {
		_ = stream.Close()
		_ = pty.Close()
	}

	go func() {
		defer closeAll()
		buf := make([]byte, 32*1024)
		for {
			n, err := pty.Read(buf)
			if n > 0 {
				payload := append([]byte(nil), buf[:n]...)
				if writeErr := shellproto.WriteFrame(stream, shellproto.KindData, payload); writeErr != nil {
					errCh <- writeErr
					return
				}
			}
			if err != nil {
				errCh <- err
				return
			}
		}
	}()

	go func() {
		defer closeAll()
		for {
			kind, payload, err := shellproto.ReadFrame(stream)
			if err != nil {
				errCh <- err
				return
			}
			switch kind {
			case shellproto.KindData:
				if _, err := pty.Write(payload); err != nil {
					errCh <- err
					return
				}
			case shellproto.KindJSON:
				var control shellproto.Control
				if err := json.Unmarshal(payload, &control); err != nil {
					errCh <- err
					return
				}
				if control.Op == shellproto.OpWinSize && control.WinSize != nil {
					if err := pty.Resize(control.WinSize.Cols, control.WinSize.Rows); err != nil {
						errCh <- err
						return
					}
				}
			}
		}
	}()

	go func() {
		err := pty.Wait()
		_ = shellproto.WriteJSON(stream, shellproto.Control{Op: shellproto.OpShellExit, OK: err == nil})
		errCh <- err
	}()

	select {
	case <-ctx.Done():
		closeAll()
		return ctx.Err()
	case err := <-errCh:
		closeAll()
		if err == nil || errors.Is(err, io.EOF) || errors.Is(err, net.ErrClosed) {
			return nil
		}
		return err
	}
}

func shellControlError(err error) *shellproto.ControlError {
	if err == nil {
		return nil
	}
	var targetNotFound shelltarget.TargetNotFoundError
	if errors.As(err, &targetNotFound) {
		return &shellproto.ControlError{
			ReasonCode: string(poc.ReasonCodeSHTargetNotFound),
			Message:    err.Error(),
		}
	}
	var targetAmbiguous shelltarget.TargetAmbiguousError
	if errors.As(err, &targetAmbiguous) {
		return &shellproto.ControlError{
			ReasonCode: string(poc.ReasonCodeSHTargetAmbiguous),
			Message:    err.Error(),
		}
	}
	if errors.Is(err, shelltarget.ErrTmuxMissing) {
		return &shellproto.ControlError{
			ReasonCode: string(poc.ReasonCodeSHTmuxMissing),
			Message:    err.Error(),
		}
	}
	return &shellproto.ControlError{
		ReasonCode: string(poc.ReasonCodeUnavailable),
		Message:    err.Error(),
	}
}

func defaultShellSessionName(value string) string {
	value = strings.TrimSpace(value)
	if value == "" {
		return "main"
	}
	return value
}

func defaultDeviceName(value string) string {
	value = strings.TrimSpace(value)
	if value != "" {
		return value
	}
	host, err := osHostname()
	if err != nil {
		return "miopunch"
	}
	return host
}

func defaultPlatform(value string) string {
	value = strings.TrimSpace(value)
	if value != "" {
		return value
	}
	return goruntime.GOOS
}

var osHostname = func() (string, error) {
	return os.Hostname()
}
