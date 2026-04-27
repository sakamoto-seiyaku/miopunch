// Copyright 2023 The frp Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

package punchdecision

import (
	"crypto/md5"
	"encoding/hex"
	"errors"
	"fmt"
	"net"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/samber/lo"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/wire"
)

const tcpReceiverDialLeadMs = 1000

// Engine derives NAT-hole responses and keeps state used to score successful
// punching behavior choices.
type Engine struct {
	analyzer *Analyzer
}

// Result contains the peer responses and scoring metadata produced by Engine.
type Result struct {
	VisitorResponse *wire.NatHoleResp
	ClientResponse  *wire.NatHoleResp

	AnalysisKey string
	// AnalyzerKey is the key used to scope analyzer records for UDP punching.
	// It is equal to AnalysisKey unless the caller supplies an extra scope key.
	AnalyzerKey string
	Mode        int
	Index       int

	// TCPAnalyzerKey is the key used to scope analyzer records for TCP punching.
	// It is empty when TCP punching is disabled for the round.
	TCPAnalyzerKey string
	TCPMode        int
	TCPIndex       int

	VisitorNATFeature *NatFeature
	ClientNATFeature  *NatFeature
}

// NewEngine creates a decision engine that keeps analyzer history for the
// supplied reserve duration.
func NewEngine(dataReserveDuration time.Duration) *Engine {
	return &Engine{analyzer: NewAnalyzer(dataReserveDuration)}
}

// AnalyzeOnce derives NAT-hole responses using a short-lived decision engine.
func AnalyzeOnce(sid string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*wire.NatHoleResp, *wire.NatHoleResp, error) {
	engine := NewEngine(24 * time.Hour)
	result, err := engine.Analyze(sid, visitor, client)
	if err != nil {
		return nil, nil, err
	}
	return result.VisitorResponse, result.ClientResponse, nil
}

// Clean removes stale analyzer history.
func (e *Engine) Clean() (int, int) {
	if e == nil || e.analyzer == nil {
		return 0, 0
	}
	return e.analyzer.Clean()
}

// ReportSuccess records a successful punching behavior for future scoring.
func (e *Engine) ReportSuccess(key string, mode, index int) {
	if e == nil || e.analyzer == nil {
		return
	}
	e.analyzer.ReportSuccess(key, mode, index)
}

func scopedAnalyzerKey(scopeKey, proto, analysisKey string) string {
	scopeKey = strings.TrimSpace(scopeKey)
	if scopeKey == "" {
		return analysisKey
	}
	proto = strings.TrimSpace(proto)
	return scopeKey + "\x00" + proto + "\x00" + analysisKey
}

// Analyze derives attempt-ready NAT-hole responses from exchanged snapshots.
func (e *Engine) Analyze(sid string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*Result, error) {
	return e.analyze(sid, "", visitor, client)
}

// AnalyzeWithScope derives attempt-ready NAT-hole responses from exchanged snapshots,
// while scoping analyzer memory by the caller-provided key.
func (e *Engine) AnalyzeWithScope(sid string, scopeKey string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*Result, error) {
	return e.analyze(sid, scopeKey, visitor, client)
}

func (e *Engine) analyze(sid string, scopeKey string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*Result, error) {
	if e == nil || e.analyzer == nil {
		return nil, errors.New("punch decision engine is nil")
	}
	if visitor == nil {
		return nil, errors.New("visitor nathole snapshot is nil")
	}
	if client == nil {
		return nil, errors.New("client nathole snapshot is nil")
	}

	cm := client
	vm := visitor

	// P3 transport: data plane config must match on both peers. No negotiation or downgrade.
	visitorProto := strings.TrimSpace(vm.Protocol)
	if visitorProto == "" {
		visitorProto = "quic"
	}
	clientProto := strings.TrimSpace(cm.Protocol)
	if clientProto == "" {
		clientProto = "quic"
	}
	if visitorProto != clientProto {
		return nil, fmt.Errorf("data plane protocol mismatch: visitor=%q client=%q", visitorProto, clientProto)
	}
	if visitorProto != "kcp" && visitorProto != "quic" {
		return nil, fmt.Errorf("unsupported data plane protocol: %q", visitorProto)
	}

	visitorNetwork, err := connectivity.ParseP2PNetwork(vm.P2PNetwork)
	if err != nil {
		return nil, fmt.Errorf("invalid visitor p2p_network: %w", err)
	}
	clientNetwork, err := connectivity.ParseP2PNetwork(cm.P2PNetwork)
	if err != nil {
		return nil, fmt.Errorf("invalid client p2p_network: %w", err)
	}
	effectiveNetwork, err := connectivity.MergeP2PNetwork(visitorNetwork, clientNetwork)
	if err != nil {
		return nil, fmt.Errorf("p2p_network mismatch: visitor=%s client=%s: %w", visitorNetwork, clientNetwork, err)
	}
	if effectiveNetwork == connectivity.P2PNetworkTCPOnly {
		missing := make([]string, 0, 2)
		if !slices.Contains(vm.Capabilities, wire.CapabilityTCPP2PV0) {
			missing = append(missing, "visitor")
		}
		if !slices.Contains(cm.Capabilities, wire.CapabilityTCPP2PV0) {
			missing = append(missing, "client")
		}
		if len(missing) > 0 {
			return nil, fmt.Errorf("tcp_only requires capability %q (missing: %s)", wire.CapabilityTCPP2PV0, strings.Join(missing, ","))
		}
	}

	var quicCC string
	var brutalUpBps, brutalDownBps uint64
	if visitorProto == "quic" {
		visitorCC := strings.TrimSpace(vm.QuicCC)
		if visitorCC == "" {
			visitorCC = "bbr"
		}
		clientCC := strings.TrimSpace(cm.QuicCC)
		if clientCC == "" {
			clientCC = "bbr"
		}
		if visitorCC != clientCC {
			return nil, fmt.Errorf("quic cc mismatch: visitor=%q client=%q", visitorCC, clientCC)
		}
		if visitorCC != "bbr" && visitorCC != "brutal" {
			return nil, fmt.Errorf("unsupported quic cc: %q", visitorCC)
		}
		quicCC = visitorCC

		if quicCC == "brutal" {
			if vm.BrutalUpBps == 0 || vm.BrutalDownBps == 0 || cm.BrutalUpBps == 0 || cm.BrutalDownBps == 0 {
				return nil, fmt.Errorf("brutal requires explicit up/down limits")
			}
			if vm.BrutalUpBps != cm.BrutalUpBps || vm.BrutalDownBps != cm.BrutalDownBps {
				return nil, fmt.Errorf(
					"brutal limits mismatch: visitor=(up=%d,down=%d) client=(up=%d,down=%d)",
					vm.BrutalUpBps, vm.BrutalDownBps, cm.BrutalUpBps, cm.BrutalDownBps,
				)
			}
			brutalUpBps = vm.BrutalUpBps
			brutalDownBps = vm.BrutalDownBps
		}
	}

	// Always exchange direct candidates snapshot (P2).
	vResp := &wire.NatHoleResp{
		TransactionID:   vm.TransactionID,
		Sid:             sid,
		Protocol:        visitorProto,
		QuicCC:          quicCC,
		BrutalUpBps:     brutalUpBps,
		BrutalDownBps:   brutalDownBps,
		P2PNetwork:      string(effectiveNetwork),
		PeerDirectAddrs: slices.Compact(cm.DirectAddrs),
	}
	cResp := &wire.NatHoleResp{
		TransactionID:   cm.TransactionID,
		Sid:             sid,
		Protocol:        visitorProto,
		QuicCC:          quicCC,
		BrutalUpBps:     brutalUpBps,
		BrutalDownBps:   brutalDownBps,
		P2PNetwork:      string(effectiveNetwork),
		PeerDirectAddrs: slices.Compact(vm.DirectAddrs),
	}
	result := &Result{
		VisitorResponse: vResp,
		ClientResponse:  cResp,
	}

	var invalid []string

	vResp.PeerTCPDirectAddrs, invalid = filterValidHostPorts(cm.TCPDirectAddrs)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid client tcp_direct_addrs: %v", sid, invalid)
	}
	vResp.PeerTCPDirectAddrs = dedupStringsInOrder(vResp.PeerTCPDirectAddrs)

	cResp.PeerTCPDirectAddrs, invalid = filterValidHostPorts(vm.TCPDirectAddrs)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid visitor tcp_direct_addrs: %v", sid, invalid)
	}
	cResp.PeerTCPDirectAddrs = dedupStringsInOrder(cResp.PeerTCPDirectAddrs)

	selectedView := ""
	selectedReason := ""
	clientMapped := cm.MappedAddrs
	visitorMapped := vm.MappedAddrs
	if cm.STUNCN != nil && cm.STUNGlobal != nil && vm.STUNCN != nil && vm.STUNGlobal != nil {
		cnAgg := aggregateSTUNView("cn", vm.STUNCN, cm.STUNCN)
		globalAgg := aggregateSTUNView("global", vm.STUNGlobal, cm.STUNGlobal)
		selectedView, selectedReason = selectSTUNView(cnAgg, globalAgg)

		switch selectedView {
		case "cn":
			clientMapped = cm.STUNCN.MappedAddrs
			visitorMapped = vm.STUNCN.MappedAddrs
		case "global":
			clientMapped = cm.STUNGlobal.MappedAddrs
			visitorMapped = vm.STUNGlobal.MappedAddrs
		default:
			// Should never happen, but keep legacy behavior.
		}

		vResp.SelectedView = selectedView
		cResp.SelectedView = selectedView
		vResp.SelectedReason = selectedReason
		cResp.SelectedReason = selectedReason

		logutil.Debugf(
			"sid [%s] stun view observations: cn(visitor avail=%v nat=%d rtt=%d ok=%d, client avail=%v nat=%d rtt=%d ok=%d) "+
				"global(visitor avail=%v nat=%d rtt=%d ok=%d, client avail=%v nat=%d rtt=%d ok=%d)",
			sid,
			vm.STUNCN.Available, vm.STUNCN.NATDifficulty, vm.STUNCN.RTTMs, vm.STUNCN.OkCount,
			cm.STUNCN.Available, cm.STUNCN.NATDifficulty, cm.STUNCN.RTTMs, cm.STUNCN.OkCount,
			vm.STUNGlobal.Available, vm.STUNGlobal.NATDifficulty, vm.STUNGlobal.RTTMs, vm.STUNGlobal.OkCount,
			cm.STUNGlobal.Available, cm.STUNGlobal.NATDifficulty, cm.STUNGlobal.RTTMs, cm.STUNGlobal.OkCount,
		)
		logutil.Debugf(
			"sid [%s] stun view arbitration: cn(avail=%v nat=%d rtt=%d ok=%d) global(avail=%v nat=%d rtt=%d ok=%d) -> selected=%s reason=%s",
			sid,
			cnAgg.available, cnAgg.natDifficulty, cnAgg.rttMs, cnAgg.okCount,
			globalAgg.available, globalAgg.natDifficulty, globalAgg.rttMs, globalAgg.okCount,
			selectedView, selectedReason,
		)
		logutil.Infof("sid [%s] selected_view=%s reason=%s", sid, selectedView, selectedReason)
	}

	clientMapped, invalid = filterValidHostPorts(clientMapped)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid client mapped_addrs: %v", sid, invalid)
	}
	visitorMapped, invalid = filterValidHostPorts(visitorMapped)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid visitor mapped_addrs: %v", sid, invalid)
	}

	tcpSelectedView := ""
	tcpSelectedReason := ""
	clientTCPMapped := cm.TCPMappedAddrs
	visitorTCPMapped := vm.TCPMappedAddrs
	if cm.TCPSTUNCN != nil && cm.TCPSTUNGlobal != nil && vm.TCPSTUNCN != nil && vm.TCPSTUNGlobal != nil {
		cnAgg := aggregateSTUNView("cn", vm.TCPSTUNCN, cm.TCPSTUNCN)
		globalAgg := aggregateSTUNView("global", vm.TCPSTUNGlobal, cm.TCPSTUNGlobal)
		tcpSelectedView, tcpSelectedReason = selectSTUNView(cnAgg, globalAgg)

		switch tcpSelectedView {
		case "cn":
			clientTCPMapped = cm.TCPSTUNCN.MappedAddrs
			visitorTCPMapped = vm.TCPSTUNCN.MappedAddrs
		case "global":
			clientTCPMapped = cm.TCPSTUNGlobal.MappedAddrs
			visitorTCPMapped = vm.TCPSTUNGlobal.MappedAddrs
		default:
			// Keep legacy behavior when view selection returns an unknown view.
		}

		vResp.TCPSelectedView = tcpSelectedView
		cResp.TCPSelectedView = tcpSelectedView
		vResp.TCPSelectedReason = tcpSelectedReason
		cResp.TCPSelectedReason = tcpSelectedReason

		logutil.Debugf(
			"sid [%s] tcp stun view arbitration -> selected=%s reason=%s",
			sid,
			tcpSelectedView,
			tcpSelectedReason,
		)
	}

	clientTCPMapped, invalid = filterValidHostPorts(clientTCPMapped)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid client tcp_mapped_addrs: %v", sid, invalid)
	}
	visitorTCPMapped, invalid = filterValidHostPorts(visitorTCPMapped)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid visitor tcp_mapped_addrs: %v", sid, invalid)
	}

	// Always exchange assisted candidates (local interface addrs) regardless of STUN
	// view selection. Assisted addrs are not STUN-derived and must not be gated by
	// cn/global arbitration or STUN availability.
	vResp.AssistedAddrs, invalid = filterValidHostPorts(cm.AssistedAddrs)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid client assisted_addrs: %v", sid, invalid)
	}
	vResp.AssistedAddrs = slices.Compact(vResp.AssistedAddrs)
	cResp.AssistedAddrs, invalid = filterValidHostPorts(vm.AssistedAddrs)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid visitor assisted_addrs: %v", sid, invalid)
	}
	cResp.AssistedAddrs = slices.Compact(cResp.AssistedAddrs)

	// Candidate addrs are STUN-derived and therefore are the only part affected by
	// cn/global selection. Compact a clone so NAT classification can still observe
	// the original repeated mapped_addrs samples.
	vResp.CandidateAddrs = slices.Compact(slices.Clone(clientMapped))
	cResp.CandidateAddrs = slices.Compact(slices.Clone(visitorMapped))

	clientTCPCandidates, invalidTCP := offsetHostPorts(clientTCPMapped, 100)
	if len(invalidTCP) > 0 {
		logutil.Debugf("sid [%s] drop invalid client tcp_candidate_addrs (+100): %v", sid, invalidTCP)
	}
	visitorTCPCandidates, invalidTCP := offsetHostPorts(visitorTCPMapped, 100)
	if len(invalidTCP) > 0 {
		logutil.Debugf("sid [%s] drop invalid visitor tcp_candidate_addrs (+100): %v", sid, invalidTCP)
	}
	vResp.TCPCandidateAddrs = dedupStringsInOrder(clientTCPCandidates)
	cResp.TCPCandidateAddrs = dedupStringsInOrder(visitorTCPCandidates)

	// Door 2 (TCP): derive punching enablement and detect behavior independently
	// from UDP punching analysis. When evidence is insufficient, disable TCP
	// punching with an explainable error and allow attempt to continue via direct
	// candidates or UDP fallback (unless tcp_only).
	vResp.TCPPunchingEnabled = false
	cResp.TCPPunchingEnabled = false
	vResp.TCPPunchingError = ""
	cResp.TCPPunchingError = ""
	vResp.TCPDetectBehavior = nil
	cResp.TCPDetectBehavior = nil

	if effectiveNetwork == connectivity.P2PNetworkUDPOnly {
		vResp.TCPPunchingError = "tcp punching disabled by p2p_network=udp_only"
		cResp.TCPPunchingError = vResp.TCPPunchingError
	} else {
		if len(clientTCPMapped) < 2 || len(visitorTCPMapped) < 2 {
			vResp.TCPPunchingError = fmt.Sprintf(
				"tcp punching disabled: insufficient tcp_mapped_addrs samples (client=%d visitor=%d)",
				len(clientTCPMapped),
				len(visitorTCPMapped),
			)
			cResp.TCPPunchingError = vResp.TCPPunchingError
		} else if len(vResp.TCPCandidateAddrs) == 0 || len(cResp.TCPCandidateAddrs) == 0 {
			vResp.TCPPunchingError = "tcp punching disabled: no usable tcp_candidate_addrs after +100 offset"
			cResp.TCPPunchingError = vResp.TCPPunchingError
		} else {
			clientLocalIPs := parseIPs(vResp.AssistedAddrs)
			cTCPNatFeature, err := ClassifyNATFeature(clientTCPMapped, clientLocalIPs)
			if err != nil {
				vResp.TCPPunchingError = fmt.Sprintf("tcp punching disabled: classify client nat feature error: %v", err)
				cResp.TCPPunchingError = vResp.TCPPunchingError
			} else {
				visitorLocalIPs := parseIPs(cResp.AssistedAddrs)
				vTCPNatFeature, err := ClassifyNATFeature(visitorTCPMapped, visitorLocalIPs)
				if err != nil {
					vResp.TCPPunchingError = fmt.Sprintf("tcp punching disabled: classify visitor nat feature error: %v", err)
					cResp.TCPPunchingError = vResp.TCPPunchingError
				} else {
					tcpAnalysisKey := natAnalysisKey("tcp", visitorTCPMapped, vTCPNatFeature, clientTCPMapped, cTCPNatFeature)
					tcpAnalyzerKey := scopedAnalyzerKey(scopeKey, "tcp", tcpAnalysisKey)
					tcpMode, tcpIndex, cBehavior, vBehavior := e.analyzer.GetRecommandBehaviors(tcpAnalyzerKey, cTCPNatFeature, vTCPNatFeature)
					result.TCPAnalyzerKey = tcpAnalyzerKey
					result.TCPMode = tcpMode
					result.TCPIndex = tcpIndex

					tcpBudgetMs := 5000
					if effectiveNetwork == connectivity.P2PNetworkTCPOnly {
						tcpBudgetMs = 10000
					}

					vSendRandomPorts := 0
					vListenRandomPorts := 0
					cSendRandomPorts := 0
					cListenRandomPorts := 0
					if tcpMode == DetectMode2 || tcpMode == DetectMode4 {
						const (
							tcpSpraySendRandomPorts   = 128
							tcpSprayListenRandomPorts = 32
						)
						if vBehavior.Role == DetectRoleSender {
							vSendRandomPorts = tcpSpraySendRandomPorts
						}
						if vBehavior.Role == DetectRoleReceiver {
							vListenRandomPorts = tcpSprayListenRandomPorts
						}
						if cBehavior.Role == DetectRoleSender {
							cSendRandomPorts = tcpSpraySendRandomPorts
						}
						if cBehavior.Role == DetectRoleReceiver {
							cListenRandomPorts = tcpSprayListenRandomPorts
						}
					}

					vReadTimeout := max(tcpBudgetMs-vBehavior.SendDelayMs, 0)
					cReadTimeout := max(tcpBudgetMs-cBehavior.SendDelayMs, 0)

					vResp.TCPPunchingEnabled = true
					cResp.TCPPunchingEnabled = true
					vResp.TCPPunchingError = ""
					cResp.TCPPunchingError = ""
					vResp.TCPDetectBehavior = &wire.TcpDetectBehavior{
						Mode:              tcpMode,
						Role:              vBehavior.Role,
						SendDelayMs:       vBehavior.SendDelayMs,
						ReadTimeoutMs:     vReadTimeout,
						SendRandomPorts:   vSendRandomPorts,
						ListenRandomPorts: vListenRandomPorts,
						CandidatePorts: offsetPortsRanges(
							getRangePorts(clientTCPMapped, cTCPNatFeature.PortsDifference, vBehavior.PortsRangeNumber),
							100,
						),
					}
					cResp.TCPDetectBehavior = &wire.TcpDetectBehavior{
						Mode:              tcpMode,
						Role:              cBehavior.Role,
						SendDelayMs:       cBehavior.SendDelayMs,
						ReadTimeoutMs:     cReadTimeout,
						SendRandomPorts:   cSendRandomPorts,
						ListenRandomPorts: cListenRandomPorts,
						CandidatePorts: offsetPortsRanges(
							getRangePorts(visitorTCPMapped, vTCPNatFeature.PortsDifference, cBehavior.PortsRangeNumber),
							100,
						),
					}
					alignTCPPunchingSendDelays(vResp.TCPDetectBehavior, cResp.TCPDetectBehavior)

					logutil.Debugf(
						"sid [%s] tcp nat: visitor=%+v client=%+v -> mode=%d index=%d visitor_role=%s client_role=%s budget_ms=%d",
						sid,
						*vTCPNatFeature,
						*cTCPNatFeature,
						tcpMode,
						tcpIndex,
						vBehavior.Role,
						cBehavior.Role,
						tcpBudgetMs,
					)
					logutil.Debugf("sid [%s] visitor tcp detect behavior: %+v", sid, vResp.TCPDetectBehavior)
					logutil.Debugf("sid [%s] client tcp detect behavior: %+v", sid, cResp.TCPDetectBehavior)
				}
			}
		}
	}

	// NAT analysis requires at least two (non-empty) mapped addrs per peer. When
	// unavailable, we still allow a best-effort punching attempt using assisted
	// candidates (e.g. same-LAN connectivity) without claiming NAT feature support.
	natAnalysisPossible := len(clientMapped) >= 2 && len(visitorMapped) >= 2
	fallbackPunching := func(msg string) *Result {
		visitorHasTargets := len(vResp.CandidateAddrs) > 0 || len(vResp.AssistedAddrs) > 0
		clientHasTargets := len(cResp.CandidateAddrs) > 0 || len(cResp.AssistedAddrs) > 0
		if !visitorHasTargets && !clientHasTargets {
			vResp.PunchingEnabled = false
			cResp.PunchingEnabled = false
			vResp.PunchingError = "punching disabled: no assisted or STUN candidates"
			cResp.PunchingError = vResp.PunchingError
			return result
		}

		// Minimal, deterministic detect behavior when NAT features cannot be analyzed.
		// Prefer the peer that actually has targets as the sender.
		visitorRole := "receiver"
		clientRole := "sender"
		if !clientHasTargets && visitorHasTargets {
			visitorRole = "sender"
			clientRole = "receiver"
		}

		vResp.PunchingEnabled = true
		cResp.PunchingEnabled = true
		vResp.DetectBehavior = wire.NatHoleDetectBehavior{
			Role:          visitorRole,
			Mode:          0,
			TTL:           0,
			SendDelayMs:   0,
			ReadTimeoutMs: 5000,
		}
		cResp.DetectBehavior = wire.NatHoleDetectBehavior{
			Role:          clientRole,
			Mode:          0,
			TTL:           0,
			SendDelayMs:   0,
			ReadTimeoutMs: 5000,
		}
		logutil.Infof("sid [%s] punching fallback: %s (selected_view=%s reason=%s)", sid, msg, selectedView, selectedReason)
		return result
	}

	if !natAnalysisPossible {
		return fallbackPunching("nat analysis unavailable"), nil
	}

	clientLocalIPs := parseIPs(vResp.AssistedAddrs)
	cNatFeature, err := ClassifyNATFeature(clientMapped, clientLocalIPs)
	if err != nil {
		return fallbackPunching(fmt.Sprintf("classify client nat feature error: %v", err)), nil
	}

	visitorLocalIPs := parseIPs(cResp.AssistedAddrs)
	vNatFeature, err := ClassifyNATFeature(visitorMapped, visitorLocalIPs)
	if err != nil {
		return fallbackPunching(fmt.Sprintf("classify visitor nat feature error: %v", err)), nil
	}

	result.ClientNATFeature = cNatFeature
	result.VisitorNATFeature = vNatFeature
	result.AnalysisKey = udpAnalysisKey(vm.MappedAddrs, vNatFeature, cm.MappedAddrs, cNatFeature)
	result.AnalyzerKey = scopedAnalyzerKey(scopeKey, "udp", result.AnalysisKey)

	mode, index, cBehavior, vBehavior := e.analyzer.GetRecommandBehaviors(result.AnalyzerKey, cNatFeature, vNatFeature)
	result.Mode = mode
	result.Index = index

	timeoutMs := max(cBehavior.SendDelayMs, vBehavior.SendDelayMs) + 5000
	if cBehavior.ListenRandomPorts > 0 || vBehavior.ListenRandomPorts > 0 {
		timeoutMs += 30000
	}

	vResp.PunchingEnabled = true
	cResp.PunchingEnabled = true

	vResp.DetectBehavior = wire.NatHoleDetectBehavior{
		Mode:              mode,
		Role:              vBehavior.Role,
		TTL:               vBehavior.TTL,
		SendDelayMs:       vBehavior.SendDelayMs,
		ReadTimeoutMs:     timeoutMs - vBehavior.SendDelayMs,
		SendRandomPorts:   vBehavior.PortsRandomNumber,
		ListenRandomPorts: vBehavior.ListenRandomPorts,
		CandidatePorts:    getRangePorts(clientMapped, cNatFeature.PortsDifference, vBehavior.PortsRangeNumber),
	}
	cResp.DetectBehavior = wire.NatHoleDetectBehavior{
		Mode:              mode,
		Role:              cBehavior.Role,
		TTL:               cBehavior.TTL,
		SendDelayMs:       cBehavior.SendDelayMs,
		ReadTimeoutMs:     timeoutMs - cBehavior.SendDelayMs,
		SendRandomPorts:   cBehavior.PortsRandomNumber,
		ListenRandomPorts: cBehavior.ListenRandomPorts,
		CandidatePorts:    getRangePorts(visitorMapped, vNatFeature.PortsDifference, cBehavior.PortsRangeNumber),
	}

	logutil.Debugf("sid [%s] visitor nat: %+v, candidateAddrs: %v; client nat: %+v, candidateAddrs: %v, protocol: %s, quic_cc: %s",
		sid, *vNatFeature, visitorMapped, *cNatFeature, clientMapped, visitorProto, quicCC)
	logutil.Debugf("sid [%s] visitor detect behavior: %+v", sid, vResp.DetectBehavior)
	logutil.Debugf("sid [%s] client detect behavior: %+v", sid, cResp.DetectBehavior)
	return result, nil
}

func alignTCPPunchingSendDelays(a, b *wire.TcpDetectBehavior) {
	alignTCPPunchingReceiverDelay(a, b)
	alignTCPPunchingReceiverDelay(b, a)
}

func alignTCPPunchingReceiverDelay(sender, receiver *wire.TcpDetectBehavior) {
	if sender == nil || receiver == nil {
		return
	}
	if sender.Role != DetectRoleSender || receiver.Role != DetectRoleReceiver {
		return
	}
	if sender.SendDelayMs <= tcpReceiverDialLeadMs {
		return
	}

	receiverDelay := sender.SendDelayMs - tcpReceiverDialLeadMs
	if receiver.SendDelayMs < receiverDelay {
		receiver.SendDelayMs = receiverDelay
	}
}

func getRangePorts(addrs []string, difference, maxNumber int) []wire.PortsRange {
	if maxNumber <= 0 {
		return nil
	}

	addr, isLast := lo.Last(addrs)
	if !isLast {
		return nil
	}
	ports := make([]wire.PortsRange, 0, 1)
	_, portStr, err := net.SplitHostPort(addr)
	if err != nil {
		return nil
	}
	port, err := strconv.Atoi(portStr)
	if err != nil {
		return nil
	}
	ports = append(ports, wire.PortsRange{
		From: max(port-difference-5, port-maxNumber, 1),
		To:   min(port+difference+5, port+maxNumber, 65535),
	})
	return ports
}

func offsetPortsRanges(ranges []wire.PortsRange, delta int) []wire.PortsRange {
	if len(ranges) == 0 || delta == 0 {
		return ranges
	}

	out := make([]wire.PortsRange, 0, len(ranges))
	for _, r := range ranges {
		from := r.From + delta
		to := r.To + delta
		if from > 65535 {
			continue
		}
		if to > 65535 {
			to = 65535
		}
		if from < 1 {
			from = 1
		}
		if from > to {
			continue
		}
		out = append(out, wire.PortsRange{From: from, To: to})
	}
	return out
}

func offsetHostPorts(addrs []string, delta int) (valid []string, invalid []string) {
	valid = make([]string, 0, len(addrs))
	invalid = make([]string, 0)

	for _, addr := range addrs {
		host, portStr, err := net.SplitHostPort(addr)
		if err != nil {
			invalid = append(invalid, addr)
			continue
		}
		port, err := strconv.Atoi(portStr)
		if err != nil {
			invalid = append(invalid, addr)
			continue
		}
		port += delta
		if port <= 0 || port > 65535 {
			invalid = append(invalid, addr)
			continue
		}
		valid = append(valid, net.JoinHostPort(host, strconv.Itoa(port)))
	}
	return valid, invalid
}

func natAnalysisKey(prefix string, visitorMapped []string, vFeature *NatFeature, clientMapped []string, cFeature *NatFeature) string {
	hash := md5.New()
	hash.Write([]byte(prefix))
	hash.Write([]byte{0})
	writeAnalysisKeyBody(hash, visitorMapped, vFeature, clientMapped, cFeature)
	return hex.EncodeToString(hash.Sum(nil))
}

func udpAnalysisKey(visitorMapped []string, vFeature *NatFeature, clientMapped []string, cFeature *NatFeature) string {
	hash := md5.New()
	writeAnalysisKeyBody(hash, visitorMapped, vFeature, clientMapped, cFeature)
	return hex.EncodeToString(hash.Sum(nil))
}

func writeAnalysisKeyBody(hash interface{ Write([]byte) (int, error) }, visitorMapped []string, vFeature *NatFeature, clientMapped []string, cFeature *NatFeature) {
	vIPs := slices.Compact(parseIPs(visitorMapped))
	if len(vIPs) > 0 {
		hash.Write([]byte(vIPs[0]))
	}
	if vFeature != nil {
		hash.Write([]byte(vFeature.NatType))
		hash.Write([]byte(vFeature.Behavior))
		hash.Write([]byte(strconv.FormatBool(vFeature.RegularPortsChange)))
	}

	cIPs := slices.Compact(parseIPs(clientMapped))
	if len(cIPs) > 0 {
		hash.Write([]byte(cIPs[0]))
	}
	if cFeature != nil {
		hash.Write([]byte(cFeature.NatType))
		hash.Write([]byte(cFeature.Behavior))
		hash.Write([]byte(strconv.FormatBool(cFeature.RegularPortsChange)))
	}
}

func parseIPs(addrs []string) []string {
	var ips []string
	for _, addr := range addrs {
		if ip, _, err := net.SplitHostPort(addr); err == nil {
			ips = append(ips, ip)
		}
	}
	return ips
}

func filterValidHostPorts(addrs []string) (valid []string, invalid []string) {
	valid = make([]string, 0, len(addrs))
	for _, addr := range addrs {
		addr = strings.TrimSpace(addr)
		if addr == "" {
			continue
		}
		if _, _, err := net.SplitHostPort(addr); err != nil {
			invalid = append(invalid, addr)
			continue
		}
		valid = append(valid, addr)
	}
	return valid, invalid
}

func dedupStringsInOrder(in []string) []string {
	if len(in) == 0 {
		return nil
	}

	seen := make(map[string]struct{}, len(in))
	out := make([]string, 0, len(in))
	for _, s := range in {
		if _, ok := seen[s]; ok {
			continue
		}
		seen[s] = struct{}{}
		out = append(out, s)
	}
	return out
}
