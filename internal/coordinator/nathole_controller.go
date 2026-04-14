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
	"fmt"
	"net"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/fatedier/golib/errors"
	"github.com/samber/lo"
	"golang.org/x/sync/errgroup"

	"github.com/miopunch/miopunch/internal/authutil"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/wire"
)

// NatHoleTimeout seconds.
var NatHoleTimeout int64 = 10

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

func (c *Controller) GenSid() string {
	t := time.Now().Unix()
	id, _ := authutil.RandID()
	return fmt.Sprintf("%d%s", t, id)
}

func (c *Controller) HandleVisitor(m *wire.NatHoleVisitor, transporter wire.MessageTransporter, visitorUser string) {
	if m.PreCheck {
		c.mu.RLock()
		cfg, ok := c.clientCfgs[m.ProxyName]
		c.mu.RUnlock()
		if !ok {
			_ = transporter.Send(c.GenNatHoleResponse(m.TransactionID, nil, fmt.Sprintf("xtcp server for [%s] doesn't exist", m.ProxyName)))
			return
		}
		if !slices.Contains(cfg.allowUsers, visitorUser) && !slices.Contains(cfg.allowUsers, "*") {
			_ = transporter.Send(c.GenNatHoleResponse(m.TransactionID, nil, fmt.Sprintf("xtcp visitor user [%s] not allowed for [%s]", visitorUser, m.ProxyName)))
			return
		}
		_ = transporter.Send(c.GenNatHoleResponse(m.TransactionID, nil, ""))
		return
	}

	sid := c.GenSid()
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
	err := func() error {
		c.mu.Lock()
		defer c.mu.Unlock()

		clientCfg, ok = c.clientCfgs[m.ProxyName]
		if !ok {
			return fmt.Errorf("xtcp server for [%s] doesn't exist", m.ProxyName)
		}
		if !authutil.ConstantTimeEqString(m.SignKey, authutil.GetAuthKey(clientCfg.sk, m.Timestamp)) {
			return fmt.Errorf("xtcp connection of [%s] auth failed", m.ProxyName)
		}
		c.sessions[sid] = session
		return nil
	}()
	if err != nil {
		logutil.Warnf("handle visitorMsg error: %v", err)
		_ = transporter.Send(c.GenNatHoleResponse(m.TransactionID, nil, err.Error()))
		return
	}
	logutil.Tracef("handle visitor message, sid [%s], server name: %s", sid, m.ProxyName)

	defer func() {
		c.mu.Lock()
		defer c.mu.Unlock()
		delete(c.sessions, sid)
	}()

	if err := errors.PanicToError(func() {
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
		// if it's sender, wait for a while to make sure the client has send the detect messages
		if vResp.PunchingEnabled && vResp.DetectBehavior.Role == "sender" {
			time.Sleep(1 * time.Second)
		}
		_ = session.visitorTransporter.Send(vResp)
		return nil
	})
	g.Go(func() error {
		// if it's sender, wait for a while to make sure the client has send the detect messages
		if cResp.PunchingEnabled && cResp.DetectBehavior.Role == "sender" {
			time.Sleep(1 * time.Second)
		}
		_ = session.clientTransporter.Send(cResp)
		return nil
	})
	_ = g.Wait()

	sleepDur := time.Duration(cResp.DetectBehavior.ReadTimeoutMs+30000) * time.Millisecond
	if !cResp.PunchingEnabled || !vResp.PunchingEnabled {
		sleepDur = 2 * time.Second
	}
	time.Sleep(sleepDur)
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
		PeerDirectAddrs: slices.Compact(cm.DirectAddrs),
	}
	cResp := &wire.NatHoleResp{
		TransactionID:   cm.TransactionID,
		Sid:             session.sid,
		Protocol:        visitorProto,
		QuicCC:          quicCC,
		BrutalUpBps:     brutalUpBps,
		BrutalDownBps:   brutalDownBps,
		PeerDirectAddrs: slices.Compact(vm.DirectAddrs),
	}

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

	// Punching is optional in P2. If we can't analyze NAT features due to missing/invalid
	// STUN-derived mapped addrs, we keep the session usable for direct paths.
	if len(clientMapped) < 2 {
		vResp.PunchingEnabled = false
		cResp.PunchingEnabled = false
		vResp.PunchingError = "client has insufficient STUN mapped_addrs"
		cResp.PunchingError = vResp.PunchingError
		return vResp, cResp, nil
	}
	if len(visitorMapped) < 2 {
		vResp.PunchingEnabled = false
		cResp.PunchingEnabled = false
		vResp.PunchingError = "visitor has insufficient STUN mapped_addrs"
		cResp.PunchingError = vResp.PunchingError
		return vResp, cResp, nil
	}

	cNatFeature, err := ClassifyNATFeature(clientMapped, parseIPs(cm.AssistedAddrs))
	if err != nil {
		vResp.PunchingEnabled = false
		cResp.PunchingEnabled = false
		vResp.PunchingError = fmt.Sprintf("classify client NAT feature error: %v", err)
		cResp.PunchingError = vResp.PunchingError
		return vResp, cResp, nil
	}

	vNatFeature, err := ClassifyNATFeature(visitorMapped, parseIPs(vm.AssistedAddrs))
	if err != nil {
		vResp.PunchingEnabled = false
		cResp.PunchingEnabled = false
		vResp.PunchingError = fmt.Sprintf("classify visitor NAT feature error: %v", err)
		cResp.PunchingError = vResp.PunchingError
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

	vResp.CandidateAddrs = slices.Compact(clientMapped)
	vResp.AssistedAddrs = slices.Compact(cm.AssistedAddrs)
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
	cResp.CandidateAddrs = slices.Compact(visitorMapped)
	cResp.AssistedAddrs = slices.Compact(vm.AssistedAddrs)
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

func parseIPs(addrs []string) []string {
	var ips []string
	for _, addr := range addrs {
		if ip, _, err := net.SplitHostPort(addr); err == nil {
			ips = append(ips, ip)
		}
	}
	return ips
}
