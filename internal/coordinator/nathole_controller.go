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

package coordinator

import (
	"context"
	"crypto/md5"
	"encoding/hex"
	stderrors "errors"
	"fmt"
	"net"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"

	goliberrors "github.com/fatedier/golib/errors"
	"github.com/samber/lo"
	"golang.org/x/sync/errgroup"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/authutil"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/wire"
)

// NatHoleTimeout seconds.
var NatHoleTimeout int64 = 10

const tcpReceiverDialLeadMs = 1000

var randID = authutil.RandID

type ClientCfg struct {
	name       string
	sk         string
	allowUsers []string
	sidCh      chan string
}

type Session struct {
	sid            string
	analysisKey    string
	recommandMode  int
	recommandIndex int

	visitorMsg         *wire.NatHoleVisitor
	visitorTransporter wire.MessageTransporter
	vResp              *wire.NatHoleResp
	vNatFeature        *NatFeature
	vBehavior          RecommandBehavior

	clientMsg         *wire.NatHoleClient
	clientTransporter wire.MessageTransporter
	cResp             *wire.NatHoleResp
	cNatFeature       *NatFeature
	cBehavior         RecommandBehavior

	notifyCh chan struct{}
}

func (s *Session) genAnalysisKey() {
	hash := md5.New()
	vIPs := slices.Compact(parseIPs(s.visitorMsg.MappedAddrs))
	if len(vIPs) > 0 {
		hash.Write([]byte(vIPs[0]))
	}
	hash.Write([]byte(s.vNatFeature.NatType))
	hash.Write([]byte(s.vNatFeature.Behavior))
	hash.Write([]byte(strconv.FormatBool(s.vNatFeature.RegularPortsChange)))

	cIPs := slices.Compact(parseIPs(s.clientMsg.MappedAddrs))
	if len(cIPs) > 0 {
		hash.Write([]byte(cIPs[0]))
	}
	hash.Write([]byte(s.cNatFeature.NatType))
	hash.Write([]byte(s.cNatFeature.Behavior))
	hash.Write([]byte(strconv.FormatBool(s.cNatFeature.RegularPortsChange)))
	s.analysisKey = hex.EncodeToString(hash.Sum(nil))
}

type Controller struct {
	clientCfgs map[string]*ClientCfg
	sessions   map[string]*Session
	analyzer   *Analyzer

	mu sync.RWMutex
}

func NewController(analysisDataReserveDuration time.Duration) (*Controller, error) {
	return &Controller{
		clientCfgs: make(map[string]*ClientCfg),
		sessions:   make(map[string]*Session),
		analyzer:   NewAnalyzer(analysisDataReserveDuration),
	}, nil
}

func (c *Controller) CleanWorker(ctx context.Context) {
	ticker := time.NewTicker(time.Hour)
	defer ticker.Stop()
	for {
		select {
		case <-ticker.C:
			start := time.Now()
			count, total := c.analyzer.Clean()
			logutil.Tracef("clean %d/%d nathole analysis data, cost %v", count, total, time.Since(start))
		case <-ctx.Done():
			return
		}
	}
}

func (c *Controller) ListenClient(name string, sk string, allowUsers []string) (chan string, error) {
	cfg := &ClientCfg{
		name:       name,
		sk:         sk,
		allowUsers: allowUsers,
		sidCh:      make(chan string),
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if _, ok := c.clientCfgs[name]; ok {
		return nil, fmt.Errorf("proxy [%s] is repeated", name)
	}
	c.clientCfgs[name] = cfg
	return cfg.sidCh, nil
}

func (c *Controller) CloseClient(name string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	delete(c.clientCfgs, name)
}

func (c *Controller) GenSid() (string, error) {
	t := time.Now().Unix()
	id, err := randID()
	if err != nil {
		return "", fmt.Errorf("generate sid random id: %w", err)
	}
	return fmt.Sprintf("%d%s", t, id), nil
}

func (c *Controller) HandleVisitor(m *wire.NatHoleVisitor, transporter wire.MessageTransporter, visitorUser string) {
	if m.PreCheck {
		c.mu.RLock()
		cfg, ok := c.clientCfgs[m.ProxyName]
		c.mu.RUnlock()
		if !ok {
			c.sendNatHoleResponse(transporter, c.GenNatHoleResponse(m.TransactionID, nil, fmt.Sprintf("proxy [%s] not found", m.ProxyName)), "visitor precheck proxy missing")
			return
		}
		if !slices.Contains(cfg.allowUsers, visitorUser) && !slices.Contains(cfg.allowUsers, "*") {
			c.sendNatHoleResponse(transporter, c.GenNatHoleResponse(m.TransactionID, nil, fmt.Sprintf("visitor user [%s] not allowed for proxy [%s]", visitorUser, m.ProxyName)), "visitor precheck user denied")
			return
		}
		c.sendNatHoleResponse(transporter, c.GenNatHoleResponse(m.TransactionID, nil, ""), "visitor precheck ok")
		return
	}

	sid, err := c.GenSid()
	if err != nil {
		logutil.Warnf("generate sid error: %v", err)
		c.sendNatHoleResponse(transporter, c.GenNatHoleResponse(m.TransactionID, nil, "generate sid failed"), "visitor sid generation failed")
		return
	}
	session := &Session{
		sid:                sid,
		visitorMsg:         m,
		visitorTransporter: transporter,
		notifyCh:           make(chan struct{}, 1),
	}
	var (
		clientCfg *ClientCfg
		ok        bool
	)
	err = func() error {
		c.mu.Lock()
		defer c.mu.Unlock()

		clientCfg, ok = c.clientCfgs[m.ProxyName]
		if !ok {
			return fmt.Errorf("proxy [%s] not found", m.ProxyName)
		}
		if !authutil.ConstantTimeEqString(m.SignKey, authutil.GetAuthKey(clientCfg.sk, m.Timestamp)) {
			return fmt.Errorf("auth failed for proxy [%s]", m.ProxyName)
		}
		c.sessions[sid] = session
		return nil
	}()
	if err != nil {
		logutil.Warnf("handle visitorMsg error: %v", err)
		c.sendNatHoleResponse(transporter, c.GenNatHoleResponse(m.TransactionID, nil, err.Error()), "visitor error response")
		return
	}
	logutil.Tracef("handle visitor message, sid [%s], server name: %s", sid, m.ProxyName)

	defer func() {
		c.mu.Lock()
		defer c.mu.Unlock()
		delete(c.sessions, sid)
	}()

	if err := goliberrors.PanicToError(func() {
		clientCfg.sidCh <- sid
	}); err != nil {
		return
	}

	// wait for NatHoleClient message
	select {
	case <-session.notifyCh:
	case <-time.After(time.Duration(NatHoleTimeout) * time.Second):
		logutil.Debugf("wait for NatHoleClient message timeout, sid [%s]", sid)
		return
	}

	// Make hole-punching decisions based on the NAT information of the client and visitor.
	vResp, cResp, err := c.analysis(session)
	if err != nil {
		logutil.Debugf("sid [%s] analysis error: %v", err)
		vResp = c.GenNatHoleResponse(session.visitorMsg.TransactionID, nil, err.Error())
		cResp = c.GenNatHoleResponse(session.clientMsg.TransactionID, nil, err.Error())
	}
	session.cResp = cResp
	session.vResp = vResp

	// send response to visitor and client
	var g errgroup.Group
	g.Go(func() error {
		// If punching is the only viable path (no peer direct addrs) and this peer
		// is the sender, delay so the receiver has time to start listening.
		if vResp.PunchingEnabled && vResp.DetectBehavior.Role == "sender" && len(vResp.PeerDirectAddrs) == 0 {
			time.Sleep(1 * time.Second)
		}
		if err := session.visitorTransporter.Send(vResp); err != nil {
			return fmt.Errorf("send visitor nathole response: %w", err)
		}
		return nil
	})
	g.Go(func() error {
		// If punching is the only viable path (no peer direct addrs) and this peer
		// is the sender, delay so the receiver has time to start listening.
		if cResp.PunchingEnabled && cResp.DetectBehavior.Role == "sender" && len(cResp.PeerDirectAddrs) == 0 {
			time.Sleep(1 * time.Second)
		}
		if err := session.clientTransporter.Send(cResp); err != nil {
			return fmt.Errorf("send client nathole response: %w", err)
		}
		return nil
	})
	if err := g.Wait(); err != nil {
		logutil.Warnf("send nathole responses error, sid [%s]: %v", sid, err)
		return
	}

	sleepDur := time.Duration(cResp.DetectBehavior.ReadTimeoutMs+30000) * time.Millisecond
	if !cResp.PunchingEnabled || !vResp.PunchingEnabled {
		sleepDur = 2 * time.Second
	}
	time.Sleep(sleepDur)
}

func (c *Controller) sendNatHoleResponse(transporter wire.MessageTransporter, resp *wire.NatHoleResp, label string) {
	if transporter == nil {
		logutil.Warnf("%s send nathole response skipped: nil transporter", label)
		return
	}
	if err := transporter.Send(resp); err != nil && !stderrors.Is(err, context.Canceled) {
		logutil.Warnf("%s send nathole response error: %v", label, err)
	}
}

func (c *Controller) HandleClient(m *wire.NatHoleClient, transporter wire.MessageTransporter) {
	c.mu.RLock()
	session, ok := c.sessions[m.Sid]
	c.mu.RUnlock()
	if !ok {
		return
	}
	logutil.Tracef("handle client message, sid [%s], server name: %s", session.sid, m.ProxyName)
	session.clientMsg = m
	session.clientTransporter = transporter
	select {
	case session.notifyCh <- struct{}{}:
	default:
	}
}

func (c *Controller) HandleReport(m *wire.NatHoleReport) {
	c.mu.RLock()
	session, ok := c.sessions[m.Sid]
	c.mu.RUnlock()
	if !ok {
		logutil.Tracef("sid [%s] report make hole success: %v, but session not found", m.Sid, m.Success)
		return
	}
	if m.Success && session.analysisKey != "" {
		c.analyzer.ReportSuccess(session.analysisKey, session.recommandMode, session.recommandIndex)
	}
	logutil.Infof("sid [%s] report make hole success: %v, mode %v, index %v",
		m.Sid, m.Success, session.recommandMode, session.recommandIndex)
}

func (c *Controller) GenNatHoleResponse(transactionID string, session *Session, errInfo string) *wire.NatHoleResp {
	var sid string
	if session != nil {
		sid = session.sid
	}
	return &wire.NatHoleResp{
		TransactionID: transactionID,
		Sid:           sid,
		Error:         errInfo,
	}
}

// analysis analyzes the NAT type and behavior of the visitor and client, then makes hole-punching decisions.
// return the response to the visitor and client.
func (c *Controller) analysis(session *Session) (*wire.NatHoleResp, *wire.NatHoleResp, error) {
	cm := session.clientMsg
	vm := session.visitorMsg

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
		return nil, nil, fmt.Errorf("data plane protocol mismatch: visitor=%q client=%q", visitorProto, clientProto)
	}
	if visitorProto != "kcp" && visitorProto != "quic" {
		return nil, nil, fmt.Errorf("unsupported data plane protocol: %q", visitorProto)
	}

	visitorNetwork, err := connectivity.ParseP2PNetwork(vm.P2PNetwork)
	if err != nil {
		return nil, nil, fmt.Errorf("invalid visitor p2p_network: %w", err)
	}
	clientNetwork, err := connectivity.ParseP2PNetwork(cm.P2PNetwork)
	if err != nil {
		return nil, nil, fmt.Errorf("invalid client p2p_network: %w", err)
	}
	effectiveNetwork, err := connectivity.MergeP2PNetwork(visitorNetwork, clientNetwork)
	if err != nil {
		return nil, nil, fmt.Errorf("p2p_network mismatch: visitor=%s client=%s: %w", visitorNetwork, clientNetwork, err)
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
			return nil, nil, fmt.Errorf("tcp_only requires capability %q (missing: %s)", wire.CapabilityTCPP2PV0, strings.Join(missing, ","))
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
			return nil, nil, fmt.Errorf("quic cc mismatch: visitor=%q client=%q", visitorCC, clientCC)
		}
		if visitorCC != "bbr" && visitorCC != "brutal" {
			return nil, nil, fmt.Errorf("unsupported quic cc: %q", visitorCC)
		}
		quicCC = visitorCC

		if quicCC == "brutal" {
			if vm.BrutalUpBps == 0 || vm.BrutalDownBps == 0 || cm.BrutalUpBps == 0 || cm.BrutalDownBps == 0 {
				return nil, nil, fmt.Errorf("brutal requires explicit up/down limits")
			}
			if vm.BrutalUpBps != cm.BrutalUpBps || vm.BrutalDownBps != cm.BrutalDownBps {
				return nil, nil, fmt.Errorf(
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
		Sid:             session.sid,
		Protocol:        visitorProto,
		QuicCC:          quicCC,
		BrutalUpBps:     brutalUpBps,
		BrutalDownBps:   brutalDownBps,
		P2PNetwork:      string(effectiveNetwork),
		PeerDirectAddrs: slices.Compact(cm.DirectAddrs),
	}
	cResp := &wire.NatHoleResp{
		TransactionID:   cm.TransactionID,
		Sid:             session.sid,
		Protocol:        visitorProto,
		QuicCC:          quicCC,
		BrutalUpBps:     brutalUpBps,
		BrutalDownBps:   brutalDownBps,
		P2PNetwork:      string(effectiveNetwork),
		PeerDirectAddrs: slices.Compact(vm.DirectAddrs),
	}

	var invalid []string

	vResp.PeerTCPDirectAddrs, invalid = filterValidHostPorts(cm.TCPDirectAddrs)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid client tcp_direct_addrs: %v", session.sid, invalid)
	}
	vResp.PeerTCPDirectAddrs = dedupStringsInOrder(vResp.PeerTCPDirectAddrs)

	cResp.PeerTCPDirectAddrs, invalid = filterValidHostPorts(vm.TCPDirectAddrs)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid visitor tcp_direct_addrs: %v", session.sid, invalid)
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
			session.sid,
			vm.STUNCN.Available, vm.STUNCN.NATDifficulty, vm.STUNCN.RTTMs, vm.STUNCN.OkCount,
			cm.STUNCN.Available, cm.STUNCN.NATDifficulty, cm.STUNCN.RTTMs, cm.STUNCN.OkCount,
			vm.STUNGlobal.Available, vm.STUNGlobal.NATDifficulty, vm.STUNGlobal.RTTMs, vm.STUNGlobal.OkCount,
			cm.STUNGlobal.Available, cm.STUNGlobal.NATDifficulty, cm.STUNGlobal.RTTMs, cm.STUNGlobal.OkCount,
		)
		logutil.Debugf(
			"sid [%s] stun view arbitration: cn(avail=%v nat=%d rtt=%d ok=%d) global(avail=%v nat=%d rtt=%d ok=%d) -> selected=%s reason=%s",
			session.sid,
			cnAgg.available, cnAgg.natDifficulty, cnAgg.rttMs, cnAgg.okCount,
			globalAgg.available, globalAgg.natDifficulty, globalAgg.rttMs, globalAgg.okCount,
			selectedView, selectedReason,
		)
		logutil.Infof("sid [%s] selected_view=%s reason=%s", session.sid, selectedView, selectedReason)
	}

	clientMapped, invalid = filterValidHostPorts(clientMapped)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid client mapped_addrs: %v", session.sid, invalid)
	}
	visitorMapped, invalid = filterValidHostPorts(visitorMapped)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid visitor mapped_addrs: %v", session.sid, invalid)
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
			session.sid,
			tcpSelectedView,
			tcpSelectedReason,
		)
	}

	clientTCPMapped, invalid = filterValidHostPorts(clientTCPMapped)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid client tcp_mapped_addrs: %v", session.sid, invalid)
	}
	visitorTCPMapped, invalid = filterValidHostPorts(visitorTCPMapped)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid visitor tcp_mapped_addrs: %v", session.sid, invalid)
	}

	// Always exchange assisted candidates (local interface addrs) regardless of STUN
	// view selection. Assisted addrs are not STUN-derived and must not be gated by
	// cn/global arbitration or STUN availability.
	vResp.AssistedAddrs, invalid = filterValidHostPorts(cm.AssistedAddrs)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid client assisted_addrs: %v", session.sid, invalid)
	}
	vResp.AssistedAddrs = slices.Compact(vResp.AssistedAddrs)
	cResp.AssistedAddrs, invalid = filterValidHostPorts(vm.AssistedAddrs)
	if len(invalid) > 0 {
		logutil.Debugf("sid [%s] drop invalid visitor assisted_addrs: %v", session.sid, invalid)
	}
	cResp.AssistedAddrs = slices.Compact(cResp.AssistedAddrs)

	// Candidate addrs are STUN-derived and therefore are the only part affected by
	// cn/global selection. Compact a clone so NAT classification can still observe
	// the original repeated mapped_addrs samples.
	vResp.CandidateAddrs = slices.Compact(slices.Clone(clientMapped))
	cResp.CandidateAddrs = slices.Compact(slices.Clone(visitorMapped))

	clientTCPCandidates, invalidTCP := offsetHostPorts(clientTCPMapped, 100)
	if len(invalidTCP) > 0 {
		logutil.Debugf("sid [%s] drop invalid client tcp_candidate_addrs (+100): %v", session.sid, invalidTCP)
	}
	visitorTCPCandidates, invalidTCP := offsetHostPorts(visitorTCPMapped, 100)
	if len(invalidTCP) > 0 {
		logutil.Debugf("sid [%s] drop invalid visitor tcp_candidate_addrs (+100): %v", session.sid, invalidTCP)
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
					mode, index, cBehavior, vBehavior := c.analyzer.GetRecommandBehaviors(tcpAnalysisKey, cTCPNatFeature, vTCPNatFeature)

					tcpBudgetMs := 5000
					if effectiveNetwork == connectivity.P2PNetworkTCPOnly {
						tcpBudgetMs = 10000
					}

					vSendRandomPorts := 0
					vListenRandomPorts := 0
					cSendRandomPorts := 0
					cListenRandomPorts := 0
					if mode == DetectMode2 || mode == DetectMode4 {
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
						Mode:              mode,
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
						Mode:              mode,
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
						session.sid,
						*vTCPNatFeature,
						*cTCPNatFeature,
						mode,
						index,
						vBehavior.Role,
						cBehavior.Role,
						tcpBudgetMs,
					)
					logutil.Debugf("sid [%s] visitor tcp detect behavior: %+v", session.sid, vResp.TCPDetectBehavior)
					logutil.Debugf("sid [%s] client tcp detect behavior: %+v", session.sid, cResp.TCPDetectBehavior)
				}
			}
		}
	}

	// NAT analysis requires at least two (non-empty) mapped addrs per peer. When
	// unavailable, we still allow a best-effort punching attempt using assisted
	// candidates (e.g. same-LAN connectivity) without claiming NAT feature support.
	natAnalysisPossible := len(clientMapped) >= 2 && len(visitorMapped) >= 2
	fallbackPunching := func(msg string) (*wire.NatHoleResp, *wire.NatHoleResp) {
		visitorHasTargets := len(vResp.CandidateAddrs) > 0 || len(vResp.AssistedAddrs) > 0
		clientHasTargets := len(cResp.CandidateAddrs) > 0 || len(cResp.AssistedAddrs) > 0
		if !visitorHasTargets && !clientHasTargets {
			vResp.PunchingEnabled = false
			cResp.PunchingEnabled = false
			vResp.PunchingError = "punching disabled: no assisted or STUN candidates"
			cResp.PunchingError = vResp.PunchingError
			return vResp, cResp
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
		logutil.Infof("sid [%s] punching fallback: %s (selected_view=%s reason=%s)", session.sid, msg, selectedView, selectedReason)
		return vResp, cResp
	}

	if !natAnalysisPossible {
		vResp, cResp = fallbackPunching("nat analysis unavailable")
		return vResp, cResp, nil
	}

	clientLocalIPs := parseIPs(vResp.AssistedAddrs)
	cNatFeature, err := ClassifyNATFeature(clientMapped, clientLocalIPs)
	if err != nil {
		vResp, cResp = fallbackPunching(fmt.Sprintf("classify client nat feature error: %v", err))
		return vResp, cResp, nil
	}

	visitorLocalIPs := parseIPs(cResp.AssistedAddrs)
	vNatFeature, err := ClassifyNATFeature(visitorMapped, visitorLocalIPs)
	if err != nil {
		vResp, cResp = fallbackPunching(fmt.Sprintf("classify visitor nat feature error: %v", err))
		return vResp, cResp, nil
	}
	session.cNatFeature = cNatFeature
	session.vNatFeature = vNatFeature
	session.genAnalysisKey()

	mode, index, cBehavior, vBehavior := c.analyzer.GetRecommandBehaviors(session.analysisKey, cNatFeature, vNatFeature)
	session.recommandMode = mode
	session.recommandIndex = index
	session.cBehavior = cBehavior
	session.vBehavior = vBehavior

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
		session.sid, *vNatFeature, visitorMapped, *cNatFeature, clientMapped, visitorProto, quicCC)
	logutil.Debugf("sid [%s] visitor detect behavior: %+v", session.sid, vResp.DetectBehavior)
	logutil.Debugf("sid [%s] client detect behavior: %+v", session.sid, cResp.DetectBehavior)
	return vResp, cResp, nil
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
	return hex.EncodeToString(hash.Sum(nil))
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
