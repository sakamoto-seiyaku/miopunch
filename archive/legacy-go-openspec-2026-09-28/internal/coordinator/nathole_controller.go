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
	stderrors "errors"
	"fmt"
	"slices"
	"sync"
	"time"

	goliberrors "github.com/fatedier/golib/errors"
	"golang.org/x/sync/errgroup"

	"github.com/miopunch/miopunch/internal/authutil"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/punchdecision"
	"github.com/miopunch/miopunch/internal/wire"
)

// NatHoleTimeout seconds.
var NatHoleTimeout int64 = 10

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

	clientMsg         *wire.NatHoleClient
	clientTransporter wire.MessageTransporter
	cResp             *wire.NatHoleResp

	notifyCh chan struct{}
}

type Controller struct {
	clientCfgs map[string]*ClientCfg
	sessions   map[string]*Session
	decisions  *punchdecision.Engine

	mu sync.RWMutex
}

func NewController(analysisDataReserveDuration time.Duration) (*Controller, error) {
	return &Controller{
		clientCfgs: make(map[string]*ClientCfg),
		sessions:   make(map[string]*Session),
		decisions:  punchdecision.NewEngine(analysisDataReserveDuration),
	}, nil
}

func (c *Controller) CleanWorker(ctx context.Context) {
	ticker := time.NewTicker(time.Hour)
	defer ticker.Stop()
	for {
		select {
		case <-ticker.C:
			start := time.Now()
			count, total := c.decisions.Clean()
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
		c.decisions.ReportSuccess(session.analysisKey, session.recommandMode, session.recommandIndex)
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

func (c *Controller) analysis(session *Session) (*wire.NatHoleResp, *wire.NatHoleResp, error) {
	result, err := c.decisions.Analyze(session.sid, session.visitorMsg, session.clientMsg)
	if err != nil {
		return nil, nil, err
	}
	session.analysisKey = result.AnalysisKey
	session.recommandMode = result.Mode
	session.recommandIndex = result.Index
	return result.VisitorResponse, result.ClientResponse, nil
}
