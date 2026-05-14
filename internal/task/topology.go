package task

import (
	"encoding/json"
	"errors"
	"math"
	"os"
	"sort"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/pocstate"
)

const TopologyFormatV0 = "miopunch.topology.v0"

// TopologySnapshot is a stable, machine-readable diagnostic snapshot of the
// node's current mainline topology view.
//
// Only top-level field names and basic types are intended to be stable for v0.
// Sub-fields may evolve as MNT-03 expands.
type TopologySnapshot struct {
	Format     string    `json:"format"`
	ObservedAt time.Time `json:"observed_at"`

	Self      TopologySelf      `json:"self"`
	Net       TopologyNet       `json:"net"`
	StateHead TopologyStateHead `json:"state_head"`

	Members []TopologyMember `json:"members"`

	Presence  TopologyPresence  `json:"presence"`
	Bootstrap TopologyBootstrap `json:"bootstrap"`
	Neighbors TopologyNeighbors `json:"neighbors"`

	Attempts []TopologyAttempt `json:"attempts"`
	Payloads []TopologyPayload `json:"payloads"`
	Recovery TopologyRecovery  `json:"recovery"`
}

type TopologySelf struct {
	PeerID string `json:"peer_id"`
	Role   string `json:"role"` // owner|admin|member|unknown

	V4Hint string `json:"v4_hint,omitempty"`
	V6Hint string `json:"v6_hint,omitempty"`
}

type TopologyNet struct {
	NetID            string   `json:"net_id,omitempty"`
	BrokersEffective []string `json:"brokers_effective,omitempty"`
}

type TopologyStateHead struct {
	GovernanceHeadB64 string `json:"governance_head_b64,omitempty"`
	DeclsHeadB64      string `json:"decls_head_b64,omitempty"`
}

type TopologyMember struct {
	PeerID string `json:"peer_id"`
	Role   string `json:"role"` // owner|admin|member|unknown

	MemberName   string `json:"member_name,omitempty"`
	PlatformHint string `json:"platform,omitempty"`
	V4Hint       string `json:"v4_hint,omitempty"`
	V6Hint       string `json:"v6_hint,omitempty"`

	Revoked bool `json:"revoked,omitempty"`
}

type TopologyPresence struct {
	OnlineWindowSec  int                    `json:"online_window_sec,omitempty"`
	HelloIntervalSec int                    `json:"hello_interval_sec,omitempty"`
	Local            *TopologyPresenceLocal `json:"local,omitempty"`
}

type TopologyPresenceLocal struct {
	PeerID          string            `json:"peer_id"`
	MessageID       string            `json:"message_id"`
	Kind            string            `json:"kind"`
	CreatedAtUnixMs int64             `json:"created_at_unix_ms"`
	StateHead       TopologyStateHead `json:"state_head"`
	V4Hint          string            `json:"v4_hint,omitempty"`
	V6Hint          string            `json:"v6_hint,omitempty"`
	Signed          bool              `json:"signed"`
	SigB64          string            `json:"sig_b64,omitempty"`
}

type TopologyBootstrap struct {
	Recommendations []TopologyPeerEvidence `json:"recommendations"`
	Attempts        []TopologyPeerEvidence `json:"attempts"`
	MoreRounds      []TopologyPeerEvidence `json:"more_rounds"`
}

type TopologyPeerEvidence struct {
	PeerID string `json:"peer_id"`
	Bucket string `json:"bucket,omitempty"`
	Reason string `json:"reason,omitempty"`
}

type TopologyNeighbors struct {
	TargetK int `json:"target_k,omitempty"`

	Selected  []TopologyNeighborSelection `json:"selected"`
	Active    []TopologyNeighborEdge      `json:"active"`
	Unhealthy []TopologyNeighborHealth    `json:"unhealthy,omitempty"`

	ReconnectAttempts  []TopologyReconnectAttempt    `json:"reconnect_attempts,omitempty"`
	Replacements       []TopologyNeighborReplacement `json:"replacements,omitempty"`
	Failures           []TopologyNeighborFailure     `json:"failures,omitempty"`
	DegreeDistribution []TopologyDegree              `json:"degree_distribution"`
}

type TopologyNeighborEdge struct {
	PeerID             string `json:"peer_id"`
	Bucket             string `json:"bucket,omitempty"`
	DataProto          string `json:"data_proto,omitempty"`  // quic|kcp|tls
	PathFamily         string `json:"path_family,omitempty"` // udp4|udp6|tcp4|tcp6|unknown
	DirectIPv4         string `json:"direct_ipv4,omitempty"`
	DirectIPv6         string `json:"direct_ipv6,omitempty"`
	LocalEndpoint      string `json:"local_endpoint,omitempty"`
	RemoteEndpoint     string `json:"remote_endpoint,omitempty"`
	PublicTuple        string `json:"public_tuple,omitempty"`
	PunchStatus        string `json:"punch_status,omitempty"`
	Port               string `json:"port,omitempty"`
	Healthy            bool   `json:"healthy"`
	LastActivityUnixMs int64  `json:"last_activity_unix_ms,omitempty"`
}

type TopologyNeighborSelection struct {
	PeerID   string `json:"peer_id"`
	Bucket   string `json:"bucket,omitempty"`
	Role     string `json:"role,omitempty"`
	Reason   string `json:"reason,omitempty"`
	Dialable bool   `json:"dialable"`
}

type TopologyNeighborHealth struct {
	PeerID             string `json:"peer_id"`
	DataProto          string `json:"data_proto,omitempty"`
	PathFamily         string `json:"path_family,omitempty"`
	LastActivityUnixMs int64  `json:"last_activity_unix_ms,omitempty"`
	CloseReason        string `json:"close_reason,omitempty"`
}

type TopologyReconnectAttempt struct {
	PeerID        string `json:"peer_id"`
	ReasonCode    string `json:"reason_code,omitempty"`
	RetryBudget   int    `json:"retry_budget,omitempty"`
	StopCondition string `json:"stop_condition,omitempty"`
}

type TopologyNeighborReplacement struct {
	OldPeerID string `json:"old_peer_id,omitempty"`
	NewPeerID string `json:"new_peer_id,omitempty"`
	Bucket    string `json:"bucket,omitempty"`
	Reason    string `json:"reason,omitempty"`
}

type TopologyNeighborFailure struct {
	PeerID         string   `json:"peer_id,omitempty"`
	Bucket         string   `json:"bucket,omitempty"`
	Stage          string   `json:"stage,omitempty"`
	ReasonCode     string   `json:"reason_code,omitempty"`
	ContactedPeers []string `json:"contacted_peers,omitempty"`
	RetryBudget    int      `json:"retry_budget,omitempty"`
	StopCondition  string   `json:"stop_condition,omitempty"`
}

type TopologyDegree struct {
	PeerID  string `json:"peer_id"`
	Active  int    `json:"active"`
	TargetK int    `json:"target_k,omitempty"`
}

type TopologyAttempt struct {
	PeerID        string                   `json:"peer_id,omitempty"`
	AttemptPath   string                   `json:"attempt_path,omitempty"`
	AttemptWay    string                   `json:"attempt_way,omitempty"`
	DataProto     string                   `json:"data_proto,omitempty"`
	PathFamily    string                   `json:"path_family,omitempty"`
	Portmap       *TopologyPortmapEvidence `json:"portmap,omitempty"`
	StartedAt     int64                    `json:"started_at_unix_ms,omitempty"`
	Outcome       string                   `json:"outcome,omitempty"` // ok|fail|timeout|unknown
	Stage         string                   `json:"stage,omitempty"`
	ReasonCode    string                   `json:"reason_code,omitempty"`
	StopCondition string                   `json:"stop_condition,omitempty"`
}

type TopologyPortmapEvidence struct {
	UDPIncluded bool `json:"udp_included"`
	UDPDirectV4 int  `json:"udp_direct_v4"`
	TCPIncluded bool `json:"tcp_included,omitempty"`
	TCPDirectV4 int  `json:"tcp_direct_v4,omitempty"`
	MethodsDone int  `json:"methods_done,omitempty"`
}

type TopologyPayload struct {
	PeerID     string `json:"peer_id,omitempty"`
	Evidence   string `json:"evidence,omitempty"` // ping=ok, marker, etc.
	ObservedAt int64  `json:"observed_at_unix_ms,omitempty"`
}

type TopologyRecovery struct {
	Events []TopologyRecoveryEvent `json:"events"`
}

type TopologyRecoveryEvent struct {
	Stage          string   `json:"stage,omitempty"`
	ReasonCode     string   `json:"reason_code,omitempty"`
	Message        string   `json:"message,omitempty"`
	ContactedPeers []string `json:"contacted_peers,omitempty"`
	RetryBudget    int      `json:"retry_budget,omitempty"`
	StopCondition  string   `json:"stop_condition,omitempty"`
}

func (m *Manager) TopologySnapshot() (TopologySnapshot, error) {
	if m == nil {
		return TopologySnapshot{}, errors.New("nil manager")
	}

	now := time.Now().UTC()

	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		return TopologySnapshot{}, err
	}

	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		return TopologySnapshot{}, err
	}

	out := TopologySnapshot{
		Format:     TopologyFormatV0,
		ObservedAt: now,
		Self: TopologySelf{
			PeerID: selfID.PeerID,
			Role:   "unknown",
		},
		Net: TopologyNet{
			BrokersEffective: []string{},
		},
		StateHead: TopologyStateHead{},
		Members:   []TopologyMember{},
		Presence:  TopologyPresence{},
		Bootstrap: TopologyBootstrap{
			Recommendations: []TopologyPeerEvidence{},
			Attempts:        []TopologyPeerEvidence{},
			MoreRounds:      []TopologyPeerEvidence{},
		},
		Neighbors: TopologyNeighbors{
			Active:             []TopologyNeighborEdge{},
			DegreeDistribution: []TopologyDegree{},
		},
		Attempts: []TopologyAttempt{},
		Payloads: []TopologyPayload{},
		Recovery: TopologyRecovery{Events: []TopologyRecoveryEvent{}},
	}

	if n, err := pocstate.LoadNet(stateDir); err == nil {
		out.Net.NetID = n.NetID
		out.Net.BrokersEffective = append([]string(nil), n.BrokersEffective...)
	} else if err != nil && !errors.Is(err, os.ErrNotExist) {
		return TopologySnapshot{}, err
	}

	st, err := m.loadState()
	if err == nil && st.Local != nil {
		local := *st.Local
		local.NormalizeDefaults()
		out.Self.V4Hint = local.V4Hint
		out.Self.V6Hint = local.V6Hint
	} else if err != nil {
		return TopologySnapshot{}, err
	}

	var head pocstate.GovernanceHeadSnapshotV1
	if h, err := pocstate.LoadGovernanceHeadSnapshot(stateDir); err == nil {
		head = h
		out.StateHead.GovernanceHeadB64 = strings.TrimSpace(h.HashB64)
		if pub := selfID.Ed25519PubB64(); pub != "" {
			out.Self.Role = roleFromGovernancePub(h, pub)
		}
	} else if err != nil && !errors.Is(err, os.ErrNotExist) {
		return TopologySnapshot{}, err
	}

	var decls pocstate.DeclsFileV0
	if f, err := pocstate.LoadDecls(stateDir); err == nil {
		decls = f
		out.StateHead.DeclsHeadB64 = strings.TrimSpace(f.DeclsHeadB64)
	} else if err != nil && !errors.Is(err, os.ErrNotExist) {
		return TopologySnapshot{}, err
	}

	out.Members = mergeMembersWithKnownPeers(membersFromDecls(head, decls), st.Peers)
	out.Neighbors.TargetK = targetNeighborK(len(out.Members))
	for _, mem := range out.Members {
		if mem.PeerID == out.Self.PeerID {
			out.Self.V4Hint = mem.V4Hint
			out.Self.V6Hint = mem.V6Hint
			if out.Self.Role == "unknown" {
				out.Self.Role = mem.Role
			}
			break
		}
	}
	memberByPeerID := topologyMembersByPeerID(out.Members)

	out.Presence.OnlineWindowSec = 120
	out.Presence.HelloIntervalSec = 30
	localPresence, err := buildLocalPresenceEvidence(now, selfID, out.StateHead, out.Self)
	if err != nil {
		return TopologySnapshot{}, err
	}
	out.Presence.Local = &localPresence

	if b, err := pocstate.LoadBootstrap(stateDir); err == nil {
		out.Bootstrap.Recommendations = topologyPeerEvidenceFromBootstrap(b.Recommendations)
		out.Bootstrap.Attempts = topologyPeerEvidenceFromBootstrap(b.Attempts)
		out.Bootstrap.MoreRounds = topologyPeerEvidenceFromBootstrap(b.MoreRounds)
	} else if err != nil && !errors.Is(err, os.ErrNotExist) {
		return TopologySnapshot{}, err
	}

	out.Neighbors.Selected = selectTopologyNeighbors(
		out.Self.PeerID,
		out.Members,
		out.Bootstrap,
		st,
		out.Neighbors.TargetK,
	)
	if m.sessions != nil {
		for _, s := range m.sessions.ListAllSummaries() {
			key := s.Key.Normalize()
			if key.RemotePeerID == "" {
				continue
			}
			if s.Healthy {
				out.Neighbors.Active = append(out.Neighbors.Active, TopologyNeighborEdge{
					PeerID:             key.RemotePeerID,
					Bucket:             topologyBucketForPeer(memberByPeerID, key.RemotePeerID),
					DataProto:          string(key.Protocol),
					PathFamily:         string(key.PathFamily),
					Healthy:            true,
					LastActivityUnixMs: s.LastActivityUnixMilli,
				})
				continue
			}
			health := TopologyNeighborHealth{
				PeerID:             key.RemotePeerID,
				DataProto:          string(key.Protocol),
				PathFamily:         string(key.PathFamily),
				LastActivityUnixMs: s.LastActivityUnixMilli,
				CloseReason:        string(s.CloseReason),
			}
			out.Neighbors.Unhealthy = append(out.Neighbors.Unhealthy, health)
			out.Neighbors.ReconnectAttempts = append(out.Neighbors.ReconnectAttempts, TopologyReconnectAttempt{
				PeerID:        key.RemotePeerID,
				ReasonCode:    string(s.CloseReason),
				RetryBudget:   1,
				StopCondition: "session_closed",
			})
		}
		sort.Slice(out.Neighbors.Active, func(i, j int) bool {
			if out.Neighbors.Active[i].PeerID != out.Neighbors.Active[j].PeerID {
				return out.Neighbors.Active[i].PeerID < out.Neighbors.Active[j].PeerID
			}
			if out.Neighbors.Active[i].DataProto != out.Neighbors.Active[j].DataProto {
				return out.Neighbors.Active[i].DataProto < out.Neighbors.Active[j].DataProto
			}
			return out.Neighbors.Active[i].PathFamily < out.Neighbors.Active[j].PathFamily
		})
		sort.Slice(out.Neighbors.Unhealthy, func(i, j int) bool {
			if out.Neighbors.Unhealthy[i].PeerID != out.Neighbors.Unhealthy[j].PeerID {
				return out.Neighbors.Unhealthy[i].PeerID < out.Neighbors.Unhealthy[j].PeerID
			}
			return out.Neighbors.Unhealthy[i].LastActivityUnixMs < out.Neighbors.Unhealthy[j].LastActivityUnixMs
		})
	}
	if out.Self.PeerID != "" {
		out.Neighbors.DegreeDistribution = append(out.Neighbors.DegreeDistribution, TopologyDegree{
			PeerID:  out.Self.PeerID,
			Active:  len(out.Neighbors.Active),
			TargetK: out.Neighbors.TargetK,
		})
	}

	out.Attempts, out.Payloads = m.topologyRuntimeEvidence()
	out.Neighbors.Failures = topologyNeighborFailures(out.Attempts, memberByPeerID)
	out.Neighbors.Replacements = topologyNeighborReplacements(out.Neighbors.Selected, out.Neighbors.Active, out.Neighbors.Unhealthy)
	out.Recovery.Events = append(out.Recovery.Events, topologyRecoveryEventsFromMembers(out.Members)...)
	out.Recovery.Events = append(out.Recovery.Events, topologyRecoveryEventsFromFailures(out.Neighbors.Failures)...)

	return out, nil
}

func buildLocalPresenceEvidence(now time.Time, selfID pocstate.Identity, stateHead TopologyStateHead, self TopologySelf) (TopologyPresenceLocal, error) {
	msgID, err := controlplane.NewMsgID()
	if err != nil {
		return TopologyPresenceLocal{}, err
	}
	msgID, err = controlplane.CanonicalizeMsgID(msgID)
	if err != nil {
		return TopologyPresenceLocal{}, err
	}

	body := struct {
		PeerID    string            `json:"peer_id"`
		StateHead TopologyStateHead `json:"state_head"`
		V4Hint    string            `json:"v4_hint,omitempty"`
		V6Hint    string            `json:"v6_hint,omitempty"`
	}{
		PeerID:    selfID.PeerID,
		StateHead: stateHead,
		V4Hint:    strings.TrimSpace(self.V4Hint),
		V6Hint:    strings.TrimSpace(self.V6Hint),
	}
	bodyJSON, err := json.Marshal(body)
	if err != nil {
		return TopologyPresenceLocal{}, err
	}

	m := controlplane.Message{
		ProtoVersion: controlplane.ProtoVersionV0,
		Route: controlplane.Route{
			DstPeerID:       selfID.PeerID,
			MsgID:           msgID,
			HopLimit:        0,
			CreatedAtUnixMs: now.UnixMilli(),
			ExpiresAtUnixMs: now.Add(2 * time.Minute).UnixMilli(),
		},
		Signed: controlplane.Signed{
			SenderPeerID: selfID.PeerID,
			Kind:         "presence",
			Body:         bodyJSON,
		},
	}
	if err := controlplane.SignV0(selfID.Ed25519Priv, &m); err != nil {
		return TopologyPresenceLocal{}, err
	}

	return TopologyPresenceLocal{
		PeerID:          selfID.PeerID,
		MessageID:       msgID,
		Kind:            "presence",
		CreatedAtUnixMs: now.UnixMilli(),
		StateHead:       stateHead,
		V4Hint:          strings.TrimSpace(self.V4Hint),
		V6Hint:          strings.TrimSpace(self.V6Hint),
		Signed:          true,
		SigB64:          m.Signed.SigB64,
	}, nil
}

func topologyPeerEvidenceFromBootstrap(in []pocstate.BootstrapPeerEvidenceV0) []TopologyPeerEvidence {
	out := make([]TopologyPeerEvidence, 0, len(in))
	for _, v := range in {
		peerID := strings.TrimSpace(v.PeerID)
		if peerID == "" {
			continue
		}
		out = append(out, TopologyPeerEvidence{
			PeerID: peerID,
			Bucket: strings.TrimSpace(v.Bucket),
			Reason: strings.TrimSpace(v.Reason),
		})
	}
	return out
}

func targetNeighborK(memberCount int) int {
	if memberCount <= 0 {
		return 0
	}
	k := int(math.Ceil(math.Log(float64(memberCount))))
	if k < 2 {
		k = 2
	}
	return k
}

func roleFromGovernancePub(head pocstate.GovernanceHeadSnapshotV1, pubB64 string) string {
	pubB64 = strings.TrimSpace(pubB64)
	if pubB64 == "" {
		return "unknown"
	}
	for _, owner := range head.SnapshotBody.Owners {
		if strings.TrimSpace(owner) == pubB64 {
			return "owner"
		}
	}
	for _, admin := range head.SnapshotBody.Admins {
		if strings.TrimSpace(admin) == pubB64 {
			return "admin"
		}
	}
	return "member"
}

func membersFromDecls(head pocstate.GovernanceHeadSnapshotV1, f pocstate.DeclsFileV0) []TopologyMember {
	approved := make(map[string]TopologyMember)
	revoked := make(map[string]struct{})

	for _, d := range f.Decls {
		switch strings.TrimSpace(d.Kind) {
		case pocstate.DeclKindApproveMember:
			var body pocstate.ApproveMemberBodyV0
			if err := json.Unmarshal(d.Body, &body); err != nil {
				continue
			}
			peerID := strings.TrimSpace(body.MemberPeerID)
			if peerID == "" {
				continue
			}

			role := "unknown"
			if strings.TrimSpace(body.Ed25519PubB64) != "" && strings.TrimSpace(head.HashB64) != "" {
				role = roleFromGovernancePub(head, body.Ed25519PubB64)
			}

			approved[peerID] = TopologyMember{
				PeerID:       peerID,
				Role:         role,
				MemberName:   strings.TrimSpace(body.MemberName),
				PlatformHint: strings.TrimSpace(body.PlatformHint),
				V4Hint:       strings.TrimSpace(body.V4Hint),
				V6Hint:       strings.TrimSpace(body.V6Hint),
				Revoked:      false,
			}
		case pocstate.DeclKindRevokeMember:
			var body pocstate.RevokeMemberBodyV0
			if err := json.Unmarshal(d.Body, &body); err != nil {
				continue
			}
			peerID := strings.TrimSpace(body.MemberPeerID)
			if peerID == "" {
				continue
			}
			revoked[peerID] = struct{}{}
		}
	}

	out := make([]TopologyMember, 0, len(approved))
	for peerID, mem := range approved {
		if _, ok := revoked[peerID]; ok {
			mem.Revoked = true
		}
		out = append(out, mem)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].PeerID < out[j].PeerID })
	return out
}

func mergeMembersWithKnownPeers(members []TopologyMember, peers map[string]pocstate.PeerConfig) []TopologyMember {
	out := append([]TopologyMember(nil), members...)
	byPeerID := make(map[string]int, len(out)+len(peers))
	for i, mem := range out {
		peerID := strings.TrimSpace(mem.PeerID)
		if peerID == "" {
			continue
		}
		byPeerID[peerID] = i
	}

	for peerID, cfg := range peers {
		peerID = strings.TrimSpace(peerID)
		if peerID == "" {
			continue
		}
		cfg.NormalizeDefaults()
		if i, ok := byPeerID[peerID]; ok {
			if out[i].V4Hint == "" {
				out[i].V4Hint = cfg.V4Hint
			}
			if out[i].V6Hint == "" {
				out[i].V6Hint = cfg.V6Hint
			}
			continue
		}
		byPeerID[peerID] = len(out)
		out = append(out, TopologyMember{
			PeerID: peerID,
			Role:   "unknown",
			V4Hint: cfg.V4Hint,
			V6Hint: cfg.V6Hint,
		})
	}

	sort.Slice(out, func(i, j int) bool { return out[i].PeerID < out[j].PeerID })
	return out
}
