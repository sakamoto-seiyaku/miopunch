package task

import (
	"encoding/json"
	"strings"

	"github.com/miopunch/miopunch/event"
)

const maxTopologyRuntimeEntries = 16

func (m *Manager) recordTopologyAttempt(attempt TopologyAttempt) {
	if m == nil {
		return
	}
	m.topologyMu.Lock()
	defer m.topologyMu.Unlock()

	m.topologyAttempts = append(m.topologyAttempts, attempt)
	if len(m.topologyAttempts) > maxTopologyRuntimeEntries {
		m.topologyAttempts = append([]TopologyAttempt(nil), m.topologyAttempts[len(m.topologyAttempts)-maxTopologyRuntimeEntries:]...)
	}
}

func (m *Manager) recordTopologyPayload(payload TopologyPayload) {
	if m == nil {
		return
	}
	m.topologyMu.Lock()
	defer m.topologyMu.Unlock()

	m.topologyPayloads = append(m.topologyPayloads, payload)
	if len(m.topologyPayloads) > maxTopologyRuntimeEntries {
		m.topologyPayloads = append([]TopologyPayload(nil), m.topologyPayloads[len(m.topologyPayloads)-maxTopologyRuntimeEntries:]...)
	}
}

func (m *Manager) topologyRuntimeEvidence() ([]TopologyAttempt, []TopologyPayload) {
	if m == nil {
		return []TopologyAttempt{}, []TopologyPayload{}
	}
	m.topologyMu.Lock()
	defer m.topologyMu.Unlock()

	attempts := append([]TopologyAttempt(nil), m.topologyAttempts...)
	payloads := append([]TopologyPayload(nil), m.topologyPayloads...)
	if attempts == nil {
		attempts = []TopologyAttempt{}
	}
	if payloads == nil {
		payloads = []TopologyPayload{}
	}
	return attempts, payloads
}

func topologyPortmapEvidenceFromEvents(raw string) *TopologyPortmapEvidence {
	var evidence TopologyPortmapEvidence
	seen := false

	for _, line := range strings.Split(raw, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}

		var ev event.Event
		if err := json.Unmarshal([]byte(line), &ev); err != nil {
			continue
		}

		switch strings.TrimSpace(ev.Name) {
		case "gather.portmap.snapshot":
			seen = true
			evidence.UDPIncluded = boolKV(ev.KVs, "included")
			evidence.UDPDirectV4 = intKV(ev.KVs, "direct_v4")
			if methodsDone := intKV(ev.KVs, "methods_done"); methodsDone > evidence.MethodsDone {
				evidence.MethodsDone = methodsDone
			}
		case "gather.tcp_portmap.snapshot":
			seen = true
			evidence.TCPIncluded = boolKV(ev.KVs, "included")
			evidence.TCPDirectV4 = intKV(ev.KVs, "direct_v4")
			if methodsDone := intKV(ev.KVs, "methods_done"); methodsDone > evidence.MethodsDone {
				evidence.MethodsDone = methodsDone
			}
		}
	}

	if !seen {
		return nil
	}
	return &evidence
}

func boolKV(kvs map[string]any, key string) bool {
	if kvs == nil {
		return false
	}
	v, ok := kvs[key]
	if !ok {
		return false
	}
	b, ok := v.(bool)
	return ok && b
}

func intKV(kvs map[string]any, key string) int {
	if kvs == nil {
		return 0
	}
	switch v := kvs[key].(type) {
	case float64:
		return int(v)
	case int:
		return v
	case int64:
		return int(v)
	case json.Number:
		n, _ := v.Int64()
		return int(n)
	default:
		return 0
	}
}
