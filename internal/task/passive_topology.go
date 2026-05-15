package task

import (
	"strings"
	"time"

	"github.com/miopunch/miopunch/dataplane"
)

type passivePeerSession struct {
	dataplane.PeerSession
	key dataplane.SessionKey
}

func (s *passivePeerSession) Key() dataplane.SessionKey {
	return s.key.Normalize()
}

func (s *passivePeerSession) SessionPathFacts() dataplane.SessionPathFacts {
	if s == nil {
		return dataplane.SessionPathFacts{}
	}
	return dataplane.PathFactsFromSession(s.PeerSession).Merge(
		dataplane.SessionPathFacts{PunchStatus: punchStatusFromAttemptPath(passiveAttemptPath(s.key.PathFamily))},
	)
}

// RegisterPassivePeerSession exposes an accepted inbound session to topology.
func (m *Manager) RegisterPassivePeerSession(peerID string, sess dataplane.PeerSession) {
	if m == nil || m.sessions == nil || sess == nil {
		return
	}
	key := passiveSessionKey(peerID, sess)
	if key.RemotePeerID == "" {
		return
	}
	m.sessions.Put(&passivePeerSession{PeerSession: sess, key: key})
}

// ClosePassivePeerSession removes an accepted inbound session from topology.
func (m *Manager) ClosePassivePeerSession(peerID string, sess dataplane.PeerSession, reason dataplane.CloseReason) {
	if m == nil || m.sessions == nil || sess == nil {
		return
	}
	key := passiveSessionKey(peerID, sess)
	if key.RemotePeerID == "" {
		return
	}
	m.sessions.Close(key, reason)
}

// RecordPassiveTopologyAttempt records an accepted inbound transport attempt.
func (m *Manager) RecordPassiveTopologyAttempt(peerID string, sess dataplane.PeerSession, attemptPath string, startedAt time.Time, outcome string) {
	if m == nil || sess == nil {
		return
	}
	key := passiveSessionKey(peerID, sess)
	if key.RemotePeerID == "" {
		return
	}
	if strings.TrimSpace(attemptPath) == "" {
		attemptPath = passiveAttemptPath(key.PathFamily)
	}
	if startedAt.IsZero() {
		startedAt = time.Now().UTC()
	}
	if strings.TrimSpace(outcome) == "" {
		outcome = "ok"
	}
	m.recordTopologyAttempt(TopologyAttempt{
		PeerID:      key.RemotePeerID,
		AttemptPath: attemptPath,
		AttemptWay:  attemptPath,
		DataProto:   string(key.Protocol),
		PathFamily:  string(key.PathFamily),
		StartedAt:   startedAt.UTC().UnixMilli(),
		Outcome:     outcome,
	})
}

// RecordPassiveTopologyPayload records inbound payload proof, such as ping=ok.
func (m *Manager) RecordPassiveTopologyPayload(peerID string, evidence string) {
	if m == nil {
		return
	}
	peerID = strings.TrimSpace(peerID)
	evidence = strings.TrimSpace(evidence)
	if peerID == "" || evidence == "" {
		return
	}
	m.recordTopologyPayload(TopologyPayload{
		PeerID:     peerID,
		Evidence:   evidence,
		ObservedAt: time.Now().UTC().UnixMilli(),
	})
}

func passiveSessionKey(peerID string, sess dataplane.PeerSession) dataplane.SessionKey {
	if sess == nil {
		return dataplane.SessionKey{}
	}
	key := sess.Key().Normalize()
	if peerID = strings.TrimSpace(peerID); peerID != "" {
		key.RemotePeerID = peerID
	}
	return key.Normalize()
}

func passiveAttemptPath(family dataplane.PathFamily) string {
	switch family {
	case dataplane.PathFamilyUDP4:
		return "passive_accept_udp4"
	case dataplane.PathFamilyUDP6:
		return "passive_accept_udp6"
	case dataplane.PathFamilyTCP4:
		return "passive_accept_tcp4"
	case dataplane.PathFamilyTCP6:
		return "passive_accept_tcp6"
	default:
		return "passive_accept_unknown"
	}
}

var _ dataplane.PeerSession = (*passivePeerSession)(nil)
