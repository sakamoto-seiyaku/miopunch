package task

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/pocstate"
)

const brokerReachabilityProbeTimeout = 5 * time.Second

func runtimeBrokerCandidates(st pocstate.State) []string {
	if st.Local != nil {
		explicit := normalizeBrokerCandidates(st.Local.MQTTBrokerEndpoints())
		if len(explicit) > 0 {
			return explicit
		}
	}
	return normalizeBrokerCandidates(builtinInviteBrokers())
}

func (m *Manager) selectReachableRuntimeBrokers(candidates []string) ([]string, []string, error) {
	return m.selectReachableBrokerSubset(candidates, 2)
}

func (m *Manager) selectReachableInviteBrokers(candidates []string, currentEffective []string) ([]string, []string, error) {
	candidates = normalizeBrokerCandidates(candidates)
	currentEffective = normalizeBrokerCandidates(currentEffective)

	preferred := make([]string, 0, len(candidates))
	fallback := make([]string, 0, len(candidates))
	currentSet := make(map[string]struct{}, len(currentEffective))
	for _, broker := range currentEffective {
		currentSet[broker] = struct{}{}
	}
	for _, broker := range candidates {
		if _, ok := currentSet[broker]; ok {
			fallback = append(fallback, broker)
			continue
		}
		preferred = append(preferred, broker)
	}

	selected, diagnostics, err := m.selectReachableInviteSubset(preferred, 2)
	if err == nil && len(selected) >= 2 {
		return selected, diagnostics, nil
	}

	need := 2 - len(selected)
	if need <= 0 {
		return selected, diagnostics, nil
	}

	fill, fillDiagnostics, fillErr := m.selectReachableInviteSubset(excludeBrokers(fallback, selected), need)
	diagnostics = append(diagnostics, fillDiagnostics...)
	selected = append(selected, fill...)
	if len(selected) > 0 {
		return selected, diagnostics, nil
	}
	if fillErr != nil {
		return nil, diagnostics, fillErr
	}
	if err != nil {
		return nil, diagnostics, err
	}
	return nil, diagnostics, fmt.Errorf("no reachable invite brokers")
}

func (m *Manager) selectReachableInviteSubset(candidates []string, limit int) ([]string, []string, error) {
	candidates = normalizeBrokerCandidates(candidates)
	if len(candidates) == 0 || limit <= 0 {
		return nil, nil, fmt.Errorf("no broker candidates")
	}

	selected := make([]string, 0, limit)
	diagnostics := make([]string, 0, len(candidates))
	failures := make([]string, 0, len(candidates))
	seen := make(map[string]struct{}, len(candidates))
	for _, candidate := range candidates {
		if len(selected) >= limit {
			break
		}

		resolveCtx, cancelResolve := context.WithTimeout(m.ctx, 2*time.Second)
		canonical, warnings, err := controlplane.CanonicalizeInviteBrokers(resolveCtx, nil, []string{candidate})
		cancelResolve()
		if err != nil {
			failure := fmt.Sprintf("%s: %v", candidate, err)
			diagnostics = append(diagnostics, "mqtt broker skipped: "+failure)
			failures = append(failures, failure)
			continue
		}
		for _, warning := range warnings {
			if strings.TrimSpace(warning) != "" {
				diagnostics = append(diagnostics, "warning: "+warning)
			}
		}

		for _, broker := range canonical {
			if len(selected) >= limit {
				break
			}
			broker = normalizeBrokerEndpoint(broker)
			if broker == "" {
				continue
			}
			if _, ok := seen[broker]; ok {
				continue
			}
			seen[broker] = struct{}{}

			probeCtx, cancel := context.WithTimeout(m.ctx, brokerReachabilityProbeTimeout)
			err := checkMQTTBrokersReachable(probeCtx, []string{broker}, "miopunch-invite-preflight")
			cancel()
			if err != nil {
				failure := fmt.Sprintf("%s: %v", broker, err)
				diagnostics = append(diagnostics, "mqtt broker skipped: "+failure)
				failures = append(failures, failure)
				continue
			}
			selected = append(selected, broker)
		}
	}

	if len(selected) == 0 {
		return nil, diagnostics, brokerFailuresError(failures, "no reachable invite brokers")
	}
	return selected, diagnostics, nil
}

func (m *Manager) selectReachableBrokerSubset(candidates []string, limit int) ([]string, []string, error) {
	candidates = normalizeBrokerCandidates(candidates)
	if len(candidates) == 0 || limit <= 0 {
		return nil, nil, fmt.Errorf("no broker candidates")
	}

	capHint := limit
	if len(candidates) < capHint {
		capHint = len(candidates)
	}
	selected := make([]string, 0, capHint)
	diagnostics := make([]string, 0, len(candidates))
	failures := make([]string, 0, len(candidates))
	for _, broker := range candidates {
		probeCtx, cancel := context.WithTimeout(m.ctx, brokerReachabilityProbeTimeout)
		err := checkMQTTBrokersReachable(probeCtx, []string{broker}, "miopunch-broker-preflight")
		cancel()
		if err != nil {
			failure := fmt.Sprintf("%s: %v", broker, err)
			diagnostics = append(diagnostics, "mqtt broker skipped: "+failure)
			failures = append(failures, failure)
			continue
		}
		selected = append(selected, broker)
		if len(selected) >= limit {
			break
		}
	}
	if len(selected) == 0 {
		return nil, diagnostics, brokerFailuresError(failures, "no reachable mqtt brokers")
	}
	return selected, diagnostics, nil
}

func excludeBrokers(candidates []string, exclude []string) []string {
	if len(candidates) == 0 {
		return nil
	}
	blocked := make(map[string]struct{}, len(exclude))
	for _, broker := range exclude {
		broker = normalizeBrokerEndpoint(broker)
		if broker != "" {
			blocked[broker] = struct{}{}
		}
	}
	out := make([]string, 0, len(candidates))
	for _, broker := range candidates {
		broker = normalizeBrokerEndpoint(broker)
		if broker == "" {
			continue
		}
		if _, ok := blocked[broker]; ok {
			continue
		}
		out = append(out, broker)
	}
	return out
}

func runtimeBrokerEndpointsForLocal(cfg *pocstate.LocalConfig) []string {
	if cfg == nil {
		return nil
	}
	return normalizeBrokerCandidates(cfg.MQTTBrokerEndpoints())
}

func runtimeBrokerEndpointsForPeer(cfg pocstate.PeerConfig) []string {
	return normalizeBrokerCandidates(cfg.MQTTBrokerEndpoints())
}

func sameBrokerList(left []string, right []string) bool {
	left = normalizeBrokerCandidates(left)
	right = normalizeBrokerCandidates(right)
	if len(left) != len(right) {
		return false
	}
	for i := range left {
		if strings.TrimSpace(left[i]) != strings.TrimSpace(right[i]) {
			return false
		}
	}
	return true
}
