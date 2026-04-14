// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

package peer

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/event"

	"github.com/miopunch/miopunch/internal/authutil"
	"github.com/miopunch/miopunch/internal/control"
	"github.com/miopunch/miopunch/internal/punching"
	"github.com/miopunch/miopunch/internal/wire"
)

type VisitorConfig struct {
	CoordAddr    string
	ControlProto control.Protocol

	Signaling string // coord | mqtt

	MQTTBroker      string
	MQTTTopicPrefix string
	MQTTUser        string
	MQTTPass        string

	User      string
	ProxyName string
	SecretKey string
	DataProto string // kcp | quic
	QuicCC    string // bbr | brutal (only when DataProto=quic)

	Payload []byte

	BuiltinDNSMode    string
	BuiltinDNSServers []string

	StunServers []string
	// StunExplicit indicates the user explicitly configured STUN (including empty).
	// When true, internal STUN defaults and cn/global arbitration are disabled.
	StunExplicit          bool
	StunTimeout           time.Duration
	GatherTimeout         time.Duration
	AttemptV6Timeout      time.Duration
	AttemptPortmapTimeout time.Duration
	P2PIPFamily           connectivity.P2PIPFamily
	DisablePortMap        bool
	P2PListenAddr         string
	DisableAssistedAddrs  bool
	HelloTimeout          time.Duration
	ExchangeInfoTimeout   time.Duration
	SessionOverallTimeout time.Duration

	Emitter *event.Emitter
}

func RunVisitor(ctx context.Context, cfg VisitorConfig) error {
	fail := func(stage event.Stage, err error, msg string, kvs map[string]any) error {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(stage, err, msg, kvs)
		}
		return err
	}

	if strings.TrimSpace(cfg.Signaling) == "" {
		cfg.Signaling = "coord"
	}
	if cfg.Signaling != "coord" && cfg.Signaling != "mqtt" {
		return fail(event.StageSupervisor, fmt.Errorf("unsupported signaling: %q", cfg.Signaling), "invalid config", nil)
	}

	if strings.TrimSpace(cfg.ProxyName) == "" {
		return fail(event.StageSupervisor, errors.New("proxy name is required"), "invalid config", nil)
	}
	if strings.TrimSpace(cfg.SecretKey) == "" {
		return fail(event.StageSupervisor, errors.New("secret key is required"), "invalid config", nil)
	}
	if cfg.DataProto == "" {
		cfg.DataProto = "quic"
	}
	if cfg.DataProto != "kcp" && cfg.DataProto != "quic" {
		return fail(event.StageSupervisor, fmt.Errorf("unsupported data proto: %q", cfg.DataProto), "invalid config", nil)
	}
	if cfg.QuicCC == "" {
		cfg.QuicCC = "bbr"
	}
	if cfg.DataProto == "quic" {
		if cfg.QuicCC != "bbr" && cfg.QuicCC != "brutal" {
			return fail(event.StageSupervisor, fmt.Errorf("unsupported quic cc: %q", cfg.QuicCC), "invalid config", nil)
		}
	} else {
		// For kcp mode, keep quic-cc empty to avoid implying a meaningful value.
		cfg.QuicCC = ""
	}
	if cfg.HelloTimeout == 0 {
		cfg.HelloTimeout = 5 * time.Second
	}
	if cfg.ExchangeInfoTimeout == 0 {
		cfg.ExchangeInfoTimeout = 5 * time.Second
	}
	if cfg.SessionOverallTimeout == 0 {
		cfg.SessionOverallTimeout = 60 * time.Second
	}
	if len(cfg.Payload) == 0 {
		cfg.Payload = []byte("ping")
	}
	family, err := connectivity.ParseP2PIPFamily(string(cfg.P2PIPFamily))
	if err != nil {
		return fail(event.StageSupervisor, err, "invalid config", map[string]any{"p2p_ip_family": cfg.P2PIPFamily})
	}
	cfg.P2PIPFamily = family

	if cfg.Signaling == "mqtt" {
		return runVisitorMQTT(ctx, cfg)
	}

	sess, err := dialHello(ctx, cfg.CoordAddr, cfg.ControlProto, &wire.PeerHello{
		Role: "visitor",
		User: cfg.User,
	}, cfg.HelloTimeout)
	if err != nil {
		return fail(event.StageSignaling, err, "connect to coordinator failed", map[string]any{
			"coord":         cfg.CoordAddr,
			"control_proto": string(cfg.ControlProto),
			"proxy_name":    cfg.ProxyName,
		})
	}
	defer sess.rwc.Close()

	if cfg.Emitter != nil {
		cfg.Emitter.OK(event.StageSignaling, "connected to coordinator", map[string]any{
			"coord":         cfg.CoordAddr,
			"control_proto": string(cfg.ControlProto),
			"proxy_name":    cfg.ProxyName,
		})
	}

	sessionCtx, cancel := context.WithTimeout(ctx, cfg.SessionOverallTimeout)
	defer cancel()

	if cfg.Emitter != nil {
		cfg.Emitter.Start(event.StageSignaling, "precheck start", map[string]any{
			"proxy_name": cfg.ProxyName,
		})
	}
	if err := punching.PreCheck(sessionCtx, sess.xport, cfg.ProxyName, cfg.ExchangeInfoTimeout); err != nil {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(event.StageSignaling, err, "precheck failed", map[string]any{
				"proxy_name": cfg.ProxyName,
			})
		}
		return err
	}
	if cfg.Emitter != nil {
		cfg.Emitter.OK(event.StageSignaling, "precheck ok", nil)
	}

	listenPort, err := listenPortFromAddr(cfg.P2PListenAddr)
	if err != nil {
		return err
	}
	gather, err := connectivity.Gather(sessionCtx, "", connectivity.GatherConfig{
		ListenPort:           listenPort,
		P2PIPFamily:          cfg.P2PIPFamily,
		DisableAssistedAddrs: cfg.DisableAssistedAddrs,
		DisablePortMap:       cfg.DisablePortMap,
		StunServers:          cfg.StunServers,
		StunExplicit:         cfg.StunExplicit,
		BuiltinDNSMode:       cfg.BuiltinDNSMode,
		BuiltinDNSServers:    cfg.BuiltinDNSServers,
		StunTimeout:          cfg.StunTimeout,
		GatherTimeout:        cfg.GatherTimeout,
		SessionLease:         connectivity.PortMapLease(cfg.SessionOverallTimeout),
		Emitter:              cfg.Emitter,
	})
	if err != nil {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(event.StageGather, err, "gather failed", nil)
		}
		return err
	}
	if gather.UDP4Conn != nil {
		defer gather.UDP4Conn.Close()
	}
	if gather.UDP6Conn != nil {
		defer gather.UDP6Conn.Close()
	}

	now := time.Now().Unix()
	transactionID := punching.NewTransactionID()

	// P3(v1): brutal parameters are intentionally fixed for lab regression.
	var brutalUpBps, brutalDownBps uint64
	if cfg.DataProto == "quic" && cfg.QuicCC == "brutal" {
		brutalUpBps = 1_000_000
		brutalDownBps = 1_000_000
	}

	natHoleVisitorMsg := &wire.NatHoleVisitor{
		TransactionID: transactionID,
		ProxyName:     cfg.ProxyName,
		Protocol:      cfg.DataProto,
		QuicCC:        cfg.QuicCC,
		BrutalUpBps:   brutalUpBps,
		BrutalDownBps: brutalDownBps,
		SignKey:       authutil.GetAuthKey(cfg.SecretKey, now),
		Timestamp:     now,
		DirectAddrs:   gather.DirectAddrs,
		MappedAddrs:   gather.MappedAddrs,
		AssistedAddrs: gather.AssistedAddrs,
		STUNCN:        gather.STUNCN,
		STUNGlobal:    gather.STUNGlobal,
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(event.StageExchange, "exchange.start", map[string]any{
			"tx":            transactionID,
			"control_proto": string(cfg.ControlProto),
			"data_proto":    cfg.DataProto,
			"quic_cc":       cfg.QuicCC,
		})
	}

	natHoleRespMsg, err := punching.ExchangeInfo(sessionCtx, sess.xport, transactionID, natHoleVisitorMsg, cfg.ExchangeInfoTimeout)
	if err != nil {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(event.StageExchange, err, "exchange.failed", map[string]any{
				"tx": transactionID,
			})
		}
		return err
	}

	if cfg.Emitter != nil {
		cfg.Emitter.OK(event.StageExchange, "exchange.ok", map[string]any{
			"sid":             natHoleRespMsg.Sid,
			"tx":              transactionID,
			"data_proto":      natHoleRespMsg.Protocol,
			"quic_cc":         natHoleRespMsg.QuicCC,
			"brutal_up":       natHoleRespMsg.BrutalUpBps,
			"brutal_down":     natHoleRespMsg.BrutalDownBps,
			"selected_view":   natHoleRespMsg.SelectedView,
			"selected_reason": natHoleRespMsg.SelectedReason,
			"peer_direct":     len(natHoleRespMsg.PeerDirectAddrs),
			"punching":        natHoleRespMsg.PunchingEnabled,
		})
	}

	attemptRes, err := connectivity.Attempt(sessionCtx, natHoleRespMsg.Sid, []byte(cfg.SecretKey), gather.UDP4Conn, gather.UDP6Conn, natHoleRespMsg, connectivity.AttemptConfig{
		AttemptV6Timeout:      cfg.AttemptV6Timeout,
		AttemptPortmapTimeout: cfg.AttemptPortmapTimeout,
		P2PIPFamily:           cfg.P2PIPFamily,
		Emitter:               cfg.Emitter,
	})
	if err != nil {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(event.StageAttempt, err, "attempt failed", map[string]any{
				"sid": natHoleRespMsg.Sid,
			})
		}
		return err
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(event.StageTransport, "data plane start", map[string]any{
			"sid":        natHoleRespMsg.Sid,
			"data_proto": natHoleRespMsg.Protocol,
			"quic_cc":    natHoleRespMsg.QuicCC,
		})
	}

	dpCfg := dataplane.Config{
		Proto:  dataplane.Protocol(natHoleRespMsg.Protocol),
		QuicCC: dataplane.QUICCC(natHoleRespMsg.QuicCC),
		Brutal: dataplane.BrutalConfig{
			UpBps:   natHoleRespMsg.BrutalUpBps,
			DownBps: natHoleRespMsg.BrutalDownBps,
		},
	}
	dataErr := dataplane.DialAndExchange(sessionCtx, dpCfg, attemptRes.Conn, attemptRes.Remote, cfg.Payload, cfg.Emitter)
	if dataErr != nil && cfg.Emitter != nil {
		cfg.Emitter.Fail(event.StageTransport, dataErr, "data plane failed", map[string]any{
			"sid":        natHoleRespMsg.Sid,
			"data_proto": natHoleRespMsg.Protocol,
		})
	}
	return dataErr
}
