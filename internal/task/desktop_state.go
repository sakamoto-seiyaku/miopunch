package task

import (
	"errors"
	"os"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

const (
	DesktopStateEventSnapshot                = "snapshot"
	DesktopStateEventTaskUpsert              = "task.upsert"
	DesktopStateEventTopologyReplace         = "topology.replace"
	DesktopStateEventPeerSessionsReplace     = "peer_sessions.replace"
	DesktopStateEventShellSessionsReplace    = "shell_sessions.replace"
	DesktopStateEventConfigReplace           = "config.replace"
	DesktopStateEventDiagnosticsReplace      = "diagnostics.replace"
	DesktopStateEventApprovalRequestsReplace = "approval_requests.replace"
)

type DesktopStatus struct {
	Version   string    `json:"version"`
	StartedAt time.Time `json:"started_at"`
	UptimeMs  int64     `json:"uptime_ms"`
	Mode      string    `json:"mode"`
}

type DesktopPeerSession struct {
	RemotePeerID       string `json:"remote_peer_id"`
	DataProto          string `json:"data_proto,omitempty"`
	SecurityID         string `json:"security_id,omitempty"`
	PathFamily         string `json:"path_family,omitempty"`
	DirectIPv4         string `json:"direct_ipv4,omitempty"`
	DirectIPv6         string `json:"direct_ipv6,omitempty"`
	LocalEndpoint      string `json:"local_endpoint,omitempty"`
	RemoteEndpoint     string `json:"remote_endpoint,omitempty"`
	PublicTuple        string `json:"public_tuple,omitempty"`
	PunchStatus        string `json:"punch_status,omitempty"`
	Port               string `json:"port,omitempty"`
	Healthy            bool   `json:"healthy"`
	LastActivityUnixMs int64  `json:"last_activity_unix_ms,omitempty"`
	ClosedAtUnixMilli  int64  `json:"closed_at_unix_ms,omitempty"`
	CloseReason        string `json:"close_reason,omitempty"`
}

type DesktopShellSession struct {
	TaskID      string         `json:"task_id"`
	PeerID      string         `json:"peer_id,omitempty"`
	Target      string         `json:"target,omitempty"`
	Session     string         `json:"session,omitempty"`
	Status      Status         `json:"status,omitempty"`
	Stage       poc.Stage      `json:"stage,omitempty"`
	ReasonCode  poc.ReasonCode `json:"reason_code,omitempty"`
	ExitCode    poc.ExitCode   `json:"exit_code,omitempty"`
	CreatedAt   time.Time      `json:"created_at,omitempty"`
	ReportReady bool           `json:"report_ready,omitempty"`
	Attachable  bool           `json:"attachable"`
}

type DesktopPeerConfig struct {
	PeerID               string   `json:"peer_id,omitempty"`
	ProxyName            string   `json:"proxy_name,omitempty"`
	TopicPrefix          string   `json:"topic_prefix,omitempty"`
	MQTTBrokers          []string `json:"mqtt_brokers,omitempty"`
	V4Hint               string   `json:"v4_hint,omitempty"`
	V6Hint               string   `json:"v6_hint,omitempty"`
	DataProto            string   `json:"data_proto,omitempty"`
	QUICCC               string   `json:"quic_cc,omitempty"`
	P2PNetwork           string   `json:"p2p_network,omitempty"`
	P2PIPFamily          string   `json:"p2p_ip_family,omitempty"`
	P2PPort              int      `json:"p2p_port,omitempty"`
	StunServers          []string `json:"stun,omitempty"`
	StunExplicit         bool     `json:"stun_explicit,omitempty"`
	DisablePortMap       bool     `json:"disable_portmap,omitempty"`
	DisableAssistedAddrs bool     `json:"disable_assisted_addrs,omitempty"`
}

type DesktopRuntimeConfig struct {
	MQTTBrokers          []string `json:"mqtt_brokers"`
	P2PNetwork           string   `json:"p2p_network"`
	P2PIPFamily          string   `json:"p2p_ip_family"`
	DataProto            string   `json:"data_proto"`
	QUICCC               string   `json:"quic_cc"`
	StunServers          []string `json:"stun"`
	StunExplicit         bool     `json:"stun_explicit"`
	DisablePortMap       bool     `json:"disable_portmap"`
	DisableAssistedAddrs bool     `json:"disable_assisted_addrs"`
}

type DesktopPreferences struct {
	DefaultShellTarget  string            `json:"default_shell_target,omitempty"`
	DefaultShellSession string            `json:"default_shell_session,omitempty"`
	LogLevel            string            `json:"log_level"`
	PeerAliases         map[string]string `json:"peer_aliases,omitempty"`
}

type DesktopSettingsConfig struct {
	Runtime     DesktopRuntimeConfig `json:"runtime"`
	Preferences DesktopPreferences   `json:"preferences"`
}

type DesktopConfigApplyStatus struct {
	Runtime             string `json:"runtime"`
	Preferences         string `json:"preferences"`
	ActivePeerSessions  int    `json:"active_peer_sessions"`
	ActiveShellSessions int    `json:"active_shell_sessions"`
	RequiresReconnect   bool   `json:"requires_reconnect"`
	RestartRequired     bool   `json:"restart_required"`
}

type DesktopNetConfig struct {
	NetID            string   `json:"net_id,omitempty"`
	BrokersEffective []string `json:"brokers_effective,omitempty"`
}

type DesktopGovernanceConfig struct {
	State               string `json:"state"`
	SelfPeerID          string `json:"self_peer_id,omitempty"`
	SelfRole            string `json:"self_role,omitempty"`
	NetID               string `json:"net_id,omitempty"`
	GovernanceHeadB64   string `json:"governance_head_b64,omitempty"`
	DeclsHeadB64        string `json:"decls_head_b64,omitempty"`
	Reason              string `json:"reason,omitempty"`
	CanInitOwner        bool   `json:"can_init_owner"`
	CanCreateNewNetwork bool   `json:"can_create_new_network"`
	CanInvite           bool   `json:"can_invite"`
	CanApprove          bool   `json:"can_approve"`
}

type DesktopConfig struct {
	Local      *DesktopPeerConfig       `json:"local,omitempty"`
	KnownPeers []DesktopPeerConfig      `json:"known_peers"`
	Net        *DesktopNetConfig        `json:"net,omitempty"`
	Governance DesktopGovernanceConfig  `json:"governance"`
	Desired    DesktopSettingsConfig    `json:"desired"`
	Effective  DesktopSettingsConfig    `json:"effective"`
	Apply      DesktopConfigApplyStatus `json:"apply"`
}

type DesktopApprovalRequest struct {
	ApproveTaskID string    `json:"approve_task_id"`
	TaskID        string    `json:"task_id,omitempty"`
	InviteID      string    `json:"invite_id,omitempty"`
	RequestMsgID  string    `json:"request_msg_id"`
	MemberPeerID  string    `json:"member_peer_id,omitempty"`
	Status        string    `json:"status,omitempty"`
	Decision      string    `json:"decision,omitempty"`
	CreatedAt     time.Time `json:"created_at,omitempty"`
	UpdatedAt     time.Time `json:"updated_at,omitempty"`
	MemberName    string    `json:"member_name,omitempty"`
	PlatformHint  string    `json:"platform,omitempty"`
	V4Hint        string    `json:"v4_hint,omitempty"`
	V6Hint        string    `json:"v6_hint,omitempty"`
}

type DesktopStateSnapshot struct {
	Rev              uint64                   `json:"rev"`
	Status           DesktopStatus            `json:"status"`
	Topology         TopologySnapshot         `json:"topology"`
	Tasks            []Task                   `json:"tasks"`
	PeerSessions     []DesktopPeerSession     `json:"peer_sessions"`
	ShellSessions    []DesktopShellSession    `json:"shell_sessions"`
	Config           DesktopConfig            `json:"config"`
	Diagnostics      []poc.Fact               `json:"diagnostics"`
	ApprovalRequests []DesktopApprovalRequest `json:"approval_requests"`
}

type DesktopStateEvent struct {
	Kind             string                   `json:"kind"`
	BaseRev          uint64                   `json:"base_rev,omitempty"`
	Rev              uint64                   `json:"rev,omitempty"`
	Snapshot         *DesktopStateSnapshot    `json:"snapshot,omitempty"`
	Task             *Task                    `json:"task,omitempty"`
	Topology         *TopologySnapshot        `json:"topology,omitempty"`
	PeerSessions     []DesktopPeerSession     `json:"peer_sessions,omitempty"`
	ShellSessions    []DesktopShellSession    `json:"shell_sessions,omitempty"`
	Config           *DesktopConfig           `json:"config,omitempty"`
	Diagnostics      []poc.Fact               `json:"diagnostics,omitempty"`
	ApprovalRequests []DesktopApprovalRequest `json:"approval_requests,omitempty"`
}

type DesktopStateSubscription struct {
	C <-chan DesktopStateEvent

	closeOnce sync.Once
	closeFn   func()
}

func (s *DesktopStateSubscription) Close() {
	if s == nil {
		return
	}
	s.closeOnce.Do(func() {
		if s.closeFn != nil {
			s.closeFn()
		}
	})
}

// SubscribeDesktopStateWithSnapshot subscribes to desktop events and returns a
// snapshot built after the subscription is registered.
func (m *Manager) SubscribeDesktopStateWithSnapshot() (*DesktopStateSubscription, DesktopStateSnapshot, error) {
	if m == nil {
		return nil, DesktopStateSnapshot{}, errors.New("nil manager")
	}

	m.desktopMu.Lock()
	defer m.desktopMu.Unlock()

	id := m.nextDesktopSubID
	m.nextDesktopSubID++
	ch := make(chan DesktopStateEvent, 1)
	m.desktopSubs[id] = ch

	snapshot, err := m.desktopStateSnapshotLocked()
	if err != nil {
		delete(m.desktopSubs, id)
		close(ch)
		return nil, DesktopStateSnapshot{}, err
	}
	sub := m.desktopStateSubscriptionLocked(id, ch)
	return sub, snapshot, nil
}

func (m *Manager) desktopStateSubscriptionLocked(id int, ch chan DesktopStateEvent) *DesktopStateSubscription {
	return &DesktopStateSubscription{
		C: ch,
		closeFn: func() {
			m.desktopMu.Lock()
			defer m.desktopMu.Unlock()
			if c, ok := m.desktopSubs[id]; ok {
				delete(m.desktopSubs, id)
				close(c)
			}
		},
	}
}

func (m *Manager) DesktopStateSnapshot() (DesktopStateSnapshot, error) {
	if m == nil {
		return DesktopStateSnapshot{}, errors.New("nil manager")
	}

	m.desktopMu.Lock()
	defer m.desktopMu.Unlock()

	return m.desktopStateSnapshotLocked()
}

func (m *Manager) desktopStateSnapshotLocked() (DesktopStateSnapshot, error) {
	topology, err := m.TopologySnapshot()
	if err != nil {
		return DesktopStateSnapshot{}, err
	}

	tasks := m.List()
	peerSessions := m.buildDesktopPeerSessions()
	shellSessions := m.buildDesktopShellSessions(tasks)
	config, err := m.buildDesktopConfig(topology)
	if err != nil {
		return DesktopStateSnapshot{}, err
	}
	approvalRequests, err := m.buildDesktopApprovalRequests(tasks)
	if err != nil {
		return DesktopStateSnapshot{}, err
	}

	snapshot := DesktopStateSnapshot{
		Rev:              m.desktopRev,
		Topology:         topology,
		Tasks:            tasks,
		PeerSessions:     peerSessions,
		ShellSessions:    shellSessions,
		Config:           config,
		ApprovalRequests: approvalRequests,
	}
	snapshot.Diagnostics = buildDesktopDiagnostics(snapshot, m.statePath, snapshot.Rev)
	return snapshot, nil
}

func (m *Manager) publishDesktopFromTaskEvent(ev Event) {
	if m == nil || ev.Task == nil {
		return
	}

	taskSnapshot := ev.Task.Clone()

	m.desktopMu.Lock()
	defer m.desktopMu.Unlock()

	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind: DesktopStateEventTaskUpsert,
		Task: &taskSnapshot,
	})

	needsTasks := isShellTaskKind(taskSnapshot.Kind) || taskSnapshot.Kind == "approve"
	if needsTasks {
		tasks := m.List()
		if isShellTaskKind(taskSnapshot.Kind) {
			m.publishDesktopStateEventLocked(DesktopStateEvent{
				Kind:          DesktopStateEventShellSessionsReplace,
				ShellSessions: m.buildDesktopShellSessions(tasks),
			})
		}
		if taskSnapshot.Kind == "approve" {
			approvalRequests, err := m.buildDesktopApprovalRequests(tasks)
			if err == nil {
				m.publishDesktopStateEventLocked(DesktopStateEvent{
					Kind:             DesktopStateEventApprovalRequestsReplace,
					ApprovalRequests: approvalRequests,
				})
			}
		}
	}

	snapshot, err := m.desktopStateSnapshotLocked()
	if err != nil {
		return
	}
	nextRev := m.nextDesktopEventRevLocked()
	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:        DesktopStateEventDiagnosticsReplace,
		Diagnostics: buildDesktopDiagnostics(snapshot, m.statePath, nextRev),
	})
}

func (m *Manager) publishDesktopConfigAndTopologyChange() {
	if m == nil {
		return
	}

	m.desktopMu.Lock()
	defer m.desktopMu.Unlock()

	snapshot, err := m.desktopStateSnapshotLocked()
	if err != nil {
		return
	}

	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:   DesktopStateEventConfigReplace,
		Config: cloneDesktopConfig(snapshot.Config),
	})
	topology := snapshot.Topology
	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:     DesktopStateEventTopologyReplace,
		Topology: &topology,
	})
	nextRev := m.nextDesktopEventRevLocked()
	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:        DesktopStateEventDiagnosticsReplace,
		Diagnostics: buildDesktopDiagnostics(snapshot, m.statePath, nextRev),
	})
}

func (m *Manager) publishDesktopApprovalRequestsChange() {
	if m == nil {
		return
	}

	m.desktopMu.Lock()
	defer m.desktopMu.Unlock()

	snapshot, err := m.desktopStateSnapshotLocked()
	if err != nil {
		return
	}
	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:             DesktopStateEventApprovalRequestsReplace,
		ApprovalRequests: snapshot.ApprovalRequests,
	})
	nextRev := m.nextDesktopEventRevLocked()
	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:        DesktopStateEventDiagnosticsReplace,
		Diagnostics: buildDesktopDiagnostics(snapshot, m.statePath, nextRev),
	})
}

func (m *Manager) publishDesktopTopologyChange() {
	if m == nil {
		return
	}

	m.desktopMu.Lock()
	defer m.desktopMu.Unlock()

	snapshot, err := m.desktopStateSnapshotLocked()
	if err != nil {
		return
	}

	topology := snapshot.Topology
	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:     DesktopStateEventTopologyReplace,
		Topology: &topology,
	})
	nextRev := m.nextDesktopEventRevLocked()
	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:        DesktopStateEventDiagnosticsReplace,
		Diagnostics: buildDesktopDiagnostics(snapshot, m.statePath, nextRev),
	})
}

func (m *Manager) publishDesktopPeerSessionsChange() {
	if m == nil {
		return
	}

	m.desktopMu.Lock()
	defer m.desktopMu.Unlock()

	snapshot, err := m.desktopStateSnapshotLocked()
	if err != nil {
		return
	}

	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:         DesktopStateEventPeerSessionsReplace,
		PeerSessions: cloneDesktopPeerSessions(snapshot.PeerSessions),
	})
	topology := snapshot.Topology
	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:     DesktopStateEventTopologyReplace,
		Topology: &topology,
	})
	nextRev := m.nextDesktopEventRevLocked()
	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:        DesktopStateEventDiagnosticsReplace,
		Diagnostics: buildDesktopDiagnostics(snapshot, m.statePath, nextRev),
	})
}

func (m *Manager) publishDesktopShellSessionsChange() {
	if m == nil {
		return
	}

	m.desktopMu.Lock()
	defer m.desktopMu.Unlock()

	tasks := m.List()
	m.publishDesktopStateEventLocked(DesktopStateEvent{
		Kind:          DesktopStateEventShellSessionsReplace,
		ShellSessions: m.buildDesktopShellSessions(tasks),
	})
}

func (m *Manager) publishDesktopStateEventLocked(ev DesktopStateEvent) {
	baseRev := m.desktopRev
	m.desktopRev++
	ev.BaseRev = baseRev
	ev.Rev = m.desktopRev

	for _, ch := range m.desktopSubs {
		sendLatestDesktopState(ch, ev)
	}
}

func (m *Manager) nextDesktopEventRevLocked() uint64 {
	return m.desktopRev + 1
}

func (m *Manager) buildDesktopPeerSessions() []DesktopPeerSession {
	if m == nil || m.sessions == nil {
		return []DesktopPeerSession{}
	}

	summaries := m.sessions.ListAllSummaries()
	out := make([]DesktopPeerSession, 0, len(summaries))
	for _, summary := range summaries {
		key := summary.Key.Normalize()
		if key.RemotePeerID == "" {
			continue
		}
		out = append(out, DesktopPeerSession{
			RemotePeerID:       key.RemotePeerID,
			DataProto:          string(key.Protocol),
			SecurityID:         key.SecurityID,
			PathFamily:         string(key.PathFamily),
			Healthy:            summary.Healthy,
			LastActivityUnixMs: summary.LastActivityUnixMilli,
			ClosedAtUnixMilli:  summary.ClosedAtUnixMilli,
			CloseReason:        string(summary.CloseReason),
		})
	}
	if out == nil {
		return []DesktopPeerSession{}
	}
	return out
}

func (m *Manager) buildDesktopShellSessions(tasks []Task) []DesktopShellSession {
	out := make([]DesktopShellSession, 0)
	for _, taskSnapshot := range tasks {
		if !isShellTaskKind(taskSnapshot.Kind) || taskSnapshot.Status == StatusDone {
			continue
		}
		out = append(out, DesktopShellSession{
			TaskID:      taskSnapshot.ID,
			PeerID:      desktopFactValue(taskSnapshot.Facts, "peer_id", "peer_id="),
			Target:      desktopFactValue(taskSnapshot.Facts, "target", "target="),
			Session:     desktopFactValue(taskSnapshot.Facts, "session", "session="),
			Status:      taskSnapshot.Status,
			Stage:       taskSnapshot.Stage,
			ReasonCode:  taskSnapshot.ReasonCode,
			ExitCode:    taskSnapshot.ExitCode,
			CreatedAt:   taskSnapshot.CreatedAt,
			ReportReady: taskSnapshot.ReportReady,
			Attachable:  m.shellAttachable(taskSnapshot.ID),
		})
	}
	if out == nil {
		return []DesktopShellSession{}
	}
	return out
}

func (m *Manager) buildDesktopApprovalRequests(tasks []Task) ([]DesktopApprovalRequest, error) {
	out := make([]DesktopApprovalRequest, 0)
	seen := make(map[string]bool)

	stateDir, err := pocstate.StateDir(m.statePath)
	if err == nil {
		store, err := controlplane.NewInviteStore(stateDir)
		if err != nil {
			return nil, err
		}
		records, err := store.ListApprovalRequests()
		if err != nil {
			return nil, err
		}
		for _, rec := range records {
			req := desktopApprovalRequestFromRecord(rec)
			if req.ApproveTaskID == "" || req.RequestMsgID == "" {
				continue
			}
			seen[req.ApproveTaskID+"/"+req.RequestMsgID] = true
			out = append(out, req)
		}
	} else if strings.TrimSpace(m.statePath) != "" {
		return nil, err
	}

	for _, taskSnapshot := range tasks {
		if taskSnapshot.Kind != "approve" || taskSnapshot.Status == StatusDone {
			continue
		}
		requestMsgID := desktopFactValue(taskSnapshot.Facts, "approval_request", "approval_request=")
		if requestMsgID == "" {
			continue
		}
		key := taskSnapshot.ID + "/" + requestMsgID
		if seen[key] {
			continue
		}
		out = append(out, DesktopApprovalRequest{
			ApproveTaskID: taskSnapshot.ID,
			TaskID:        taskSnapshot.ID,
			InviteID:      desktopFactValue(taskSnapshot.Facts, "invite_id", "invite_id="),
			RequestMsgID:  requestMsgID,
			MemberPeerID:  desktopFactValue(taskSnapshot.Facts, "member_peer_id", "member_peer_id="),
			Status:        controlplane.ApprovalStatusPending,
			CreatedAt:     taskSnapshot.CreatedAt,
		})
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].CreatedAt.Equal(out[j].CreatedAt) {
			return out[i].RequestMsgID < out[j].RequestMsgID
		}
		return out[i].CreatedAt.Before(out[j].CreatedAt)
	})
	if out == nil {
		return []DesktopApprovalRequest{}, nil
	}
	return out, nil
}

func desktopApprovalRequestFromRecord(rec controlplane.ApprovalRequestRecord) DesktopApprovalRequest {
	return DesktopApprovalRequest{
		ApproveTaskID: strings.TrimSpace(rec.ApproveTaskID),
		TaskID:        strings.TrimSpace(rec.ApproveTaskID),
		InviteID:      strings.TrimSpace(rec.InviteID),
		RequestMsgID:  strings.TrimSpace(rec.RequestMsgID),
		MemberPeerID:  strings.TrimSpace(rec.MemberPeerID),
		Status:        strings.TrimSpace(rec.Status),
		Decision:      strings.TrimSpace(rec.Decision),
		CreatedAt:     timeFromUnixMilli(rec.CreatedAtUnixMs),
		UpdatedAt:     timeFromUnixMilli(rec.UpdatedAtUnixMs),
		MemberName:    strings.TrimSpace(rec.MemberName),
		PlatformHint:  strings.TrimSpace(rec.PlatformHint),
		V4Hint:        strings.TrimSpace(rec.V4Hint),
		V6Hint:        strings.TrimSpace(rec.V6Hint),
	}
}

func timeFromUnixMilli(unixMs int64) time.Time {
	if unixMs <= 0 {
		return time.Time{}
	}
	return time.UnixMilli(unixMs).UTC()
}

func (m *Manager) buildDesktopConfig(topology TopologySnapshot) (DesktopConfig, error) {
	stateSnapshot, err := m.loadState()
	if err != nil {
		return DesktopConfig{}, err
	}
	desktopSettings, err := m.loadDesktopSettings()
	if err != nil {
		return DesktopConfig{}, err
	}

	out := DesktopConfig{
		KnownPeers: []DesktopPeerConfig{},
	}

	if stateSnapshot.Local != nil {
		local := *stateSnapshot.Local
		local.NormalizeDefaults()
		if strings.TrimSpace(local.PeerID) == "" {
			local.PeerID = strings.TrimSpace(topology.Self.PeerID)
		}
		out.Local = desktopPeerConfigFromLocal(local)
		out.Desired.Runtime = desktopRuntimeConfigFromLocal(&local)
		out.Effective.Runtime = desktopRuntimeConfigFromLocal(&local)
	} else {
		out.Desired.Runtime = desktopRuntimeConfigFromLocal(nil)
		out.Effective.Runtime = desktopRuntimeConfigFromLocal(nil)
	}
	out.Desired.Preferences = desktopSettings.Preferences
	out.Effective.Preferences = desktopSettings.Preferences

	peerIDs := make([]string, 0, len(stateSnapshot.Peers))
	for peerID := range stateSnapshot.Peers {
		peerIDs = append(peerIDs, peerID)
	}
	sort.Strings(peerIDs)
	for _, peerID := range peerIDs {
		cfg := stateSnapshot.Peers[peerID]
		cfg.NormalizeDefaults()
		out.KnownPeers = append(out.KnownPeers, desktopPeerConfigFromPeer(peerID, cfg))
	}

	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		if strings.TrimSpace(m.statePath) == "" {
			return out, nil
		}
		return DesktopConfig{}, err
	}

	netState, err := pocstate.LoadNet(stateDir)
	switch {
	case err == nil:
		out.Net = &DesktopNetConfig{
			NetID:            strings.TrimSpace(netState.NetID),
			BrokersEffective: append([]string(nil), netState.BrokersEffective...),
		}
	case errors.Is(err, os.ErrNotExist):
		out.Net = nil
	default:
		return DesktopConfig{}, err
	}

	if selfID, err := pocstate.EnsureIdentity(stateDir); err == nil {
		out.Governance = desktopGovernanceConfigFromCapability(m.localGovernanceCapability(stateDir, selfID))
	} else {
		out.Governance = DesktopGovernanceConfig{
			State:  GovernanceStateForeignOrStaleNetwork,
			Reason: "ensure identity: " + err.Error(),
		}
	}

	out.Apply = m.desktopConfigApplyStatus()
	return out, nil
}

func (m *Manager) desktopConfigApplyStatus() DesktopConfigApplyStatus {
	activePeerSessions := 0
	if m != nil && m.sessions != nil {
		for _, summary := range m.sessions.ListAllSummaries() {
			if summary.Healthy {
				activePeerSessions++
			}
		}
	}

	activeShellSessions := 0
	for _, taskSnapshot := range m.List() {
		if isShellTaskKind(taskSnapshot.Kind) && taskSnapshot.Status != StatusDone {
			activeShellSessions++
		}
	}

	runtimeStatus := "immediate"
	requiresReconnect := false
	if activePeerSessions > 0 || activeShellSessions > 0 {
		runtimeStatus = "future_connections"
		requiresReconnect = true
	}
	return DesktopConfigApplyStatus{
		Runtime:             runtimeStatus,
		Preferences:         "immediate",
		ActivePeerSessions:  activePeerSessions,
		ActiveShellSessions: activeShellSessions,
		RequiresReconnect:   requiresReconnect,
		RestartRequired:     false,
	}
}

func buildDesktopDiagnostics(snapshot DesktopStateSnapshot, statePath string, rev uint64) []poc.Fact {
	activePeerSessions := 0
	closedPeerSessions := 0
	for _, session := range snapshot.PeerSessions {
		if session.Healthy {
			activePeerSessions++
			continue
		}
		closedPeerSessions++
	}

	runningTasks := 0
	for _, taskSnapshot := range snapshot.Tasks {
		if taskSnapshot.Status != StatusDone {
			runningTasks++
		}
	}

	out := []poc.Fact{
		{Message: "desktop_runtime_rev=" + uint64String(rev)},
		{Message: "self_peer_id=" + strings.TrimSpace(snapshot.Topology.Self.PeerID)},
		{Message: "self_role=" + strings.TrimSpace(snapshot.Topology.Self.Role)},
		{Message: "governance_state=" + strings.TrimSpace(snapshot.Config.Governance.State)},
		{Message: "known_peers=" + uint64String(uint64(len(snapshot.Config.KnownPeers)))},
		{Message: "active_peer_sessions=" + uint64String(uint64(activePeerSessions))},
		{Message: "closed_peer_sessions=" + uint64String(uint64(closedPeerSessions))},
		{Message: "shell_sessions=" + uint64String(uint64(len(snapshot.ShellSessions)))},
		{Message: "running_tasks=" + uint64String(uint64(runningTasks))},
		{Message: "approval_requests=" + uint64String(uint64(len(snapshot.ApprovalRequests)))},
	}
	if strings.TrimSpace(statePath) != "" {
		out = append(out, poc.Fact{Message: "state_path=" + strings.TrimSpace(statePath)})
	}
	if snapshot.Config.Net != nil && strings.TrimSpace(snapshot.Config.Net.NetID) != "" {
		out = append(out, poc.Fact{Message: "net_id=" + strings.TrimSpace(snapshot.Config.Net.NetID)})
	}
	if out == nil {
		return []poc.Fact{}
	}
	return out
}

func desktopPeerConfigFromLocal(cfg pocstate.LocalConfig) *DesktopPeerConfig {
	cfg.NormalizeDefaults()
	return &DesktopPeerConfig{
		PeerID:               strings.TrimSpace(cfg.PeerID),
		ProxyName:            strings.TrimSpace(cfg.ProxyName),
		TopicPrefix:          strings.TrimSpace(cfg.TopicPrefix),
		MQTTBrokers:          append([]string(nil), cfg.MQTTBrokerEndpoints()...),
		V4Hint:               strings.TrimSpace(cfg.V4Hint),
		V6Hint:               strings.TrimSpace(cfg.V6Hint),
		DataProto:            strings.TrimSpace(cfg.DataProto),
		QUICCC:               strings.TrimSpace(cfg.QUICCC),
		P2PNetwork:           strings.TrimSpace(cfg.P2PNetwork),
		P2PIPFamily:          strings.TrimSpace(cfg.P2PIPFamily),
		P2PPort:              cfg.P2PPort,
		StunServers:          append([]string(nil), cfg.StunServers...),
		StunExplicit:         cfg.StunExplicit,
		DisablePortMap:       cfg.DisablePortMap,
		DisableAssistedAddrs: cfg.DisableAssistedAddrs,
	}
}

func desktopGovernanceConfigFromCapability(cap localGovernanceCapability) DesktopGovernanceConfig {
	return DesktopGovernanceConfig{
		State:               strings.TrimSpace(cap.State),
		SelfPeerID:          strings.TrimSpace(cap.SelfPeerID),
		SelfRole:            strings.TrimSpace(cap.SelfRole),
		NetID:               strings.TrimSpace(cap.NetID),
		GovernanceHeadB64:   strings.TrimSpace(cap.GovernanceHeadB64),
		DeclsHeadB64:        strings.TrimSpace(cap.DeclsHeadB64),
		Reason:              strings.TrimSpace(cap.Reason),
		CanInitOwner:        cap.CanInitOwner,
		CanCreateNewNetwork: cap.CanCreateNewNetwork,
		CanInvite:           cap.CanInvite,
		CanApprove:          cap.CanApprove,
	}
}

func desktopPeerConfigFromPeer(peerID string, cfg pocstate.PeerConfig) DesktopPeerConfig {
	cfg.NormalizeDefaults()
	return DesktopPeerConfig{
		PeerID:               strings.TrimSpace(peerID),
		ProxyName:            strings.TrimSpace(cfg.ProxyName),
		TopicPrefix:          strings.TrimSpace(cfg.TopicPrefix),
		MQTTBrokers:          append([]string(nil), cfg.MQTTBrokerEndpoints()...),
		V4Hint:               strings.TrimSpace(cfg.V4Hint),
		V6Hint:               strings.TrimSpace(cfg.V6Hint),
		DataProto:            strings.TrimSpace(cfg.DataProto),
		QUICCC:               strings.TrimSpace(cfg.QUICCC),
		P2PNetwork:           strings.TrimSpace(cfg.P2PNetwork),
		P2PIPFamily:          strings.TrimSpace(cfg.P2PIPFamily),
		P2PPort:              cfg.P2PPort,
		StunServers:          append([]string(nil), cfg.StunServers...),
		StunExplicit:         cfg.StunExplicit,
		DisablePortMap:       cfg.DisablePortMap,
		DisableAssistedAddrs: cfg.DisableAssistedAddrs,
	}
}

func desktopFactValue(facts []poc.Fact, termID string, prefix string) string {
	for _, fact := range facts {
		if termID != "" && strings.TrimSpace(fact.TermID) == termID {
			if value := strings.TrimSpace(fact.Message); value != "" {
				if prefix != "" && strings.HasPrefix(value, prefix) {
					return strings.TrimSpace(strings.TrimPrefix(value, prefix))
				}
				return value
			}
		}

		message := strings.TrimSpace(fact.Message)
		if prefix != "" && strings.HasPrefix(message, prefix) {
			return strings.TrimSpace(strings.TrimPrefix(message, prefix))
		}
	}
	return ""
}

func cloneDesktopConfig(cfg DesktopConfig) *DesktopConfig {
	out := cfg
	if cfg.Local != nil {
		local := *cfg.Local
		local.MQTTBrokers = append([]string(nil), cfg.Local.MQTTBrokers...)
		local.StunServers = append([]string(nil), cfg.Local.StunServers...)
		out.Local = &local
	}
	out.KnownPeers = append([]DesktopPeerConfig(nil), cfg.KnownPeers...)
	for idx := range out.KnownPeers {
		out.KnownPeers[idx].MQTTBrokers = append([]string(nil), out.KnownPeers[idx].MQTTBrokers...)
		out.KnownPeers[idx].StunServers = append([]string(nil), out.KnownPeers[idx].StunServers...)
	}
	if cfg.Net != nil {
		netSnapshot := *cfg.Net
		netSnapshot.BrokersEffective = append([]string(nil), cfg.Net.BrokersEffective...)
		out.Net = &netSnapshot
	}
	out.Desired.Runtime.MQTTBrokers = append([]string(nil), cfg.Desired.Runtime.MQTTBrokers...)
	out.Desired.Runtime.StunServers = append([]string(nil), cfg.Desired.Runtime.StunServers...)
	out.Desired.Preferences.PeerAliases = cloneStringMap(cfg.Desired.Preferences.PeerAliases)
	out.Effective.Runtime.MQTTBrokers = append([]string(nil), cfg.Effective.Runtime.MQTTBrokers...)
	out.Effective.Runtime.StunServers = append([]string(nil), cfg.Effective.Runtime.StunServers...)
	out.Effective.Preferences.PeerAliases = cloneStringMap(cfg.Effective.Preferences.PeerAliases)
	return &out
}

func cloneDesktopPeerSessions(in []DesktopPeerSession) []DesktopPeerSession {
	return append([]DesktopPeerSession(nil), in...)
}

func cloneFacts(in []poc.Fact) []poc.Fact {
	return append([]poc.Fact(nil), in...)
}

func sendLatestDesktopState(ch chan DesktopStateEvent, ev DesktopStateEvent) {
	select {
	case ch <- ev:
		return
	default:
	}

	select {
	case <-ch:
	default:
	}
	select {
	case ch <- ev:
	default:
	}
}

func isShellTaskKind(kind string) bool {
	switch strings.TrimSpace(kind) {
	case "sh_attach", "sh_ls":
		return true
	default:
		return false
	}
}

func uint64String(v uint64) string {
	return strconv.FormatUint(v, 10)
}
