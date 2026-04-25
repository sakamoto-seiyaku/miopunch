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

	"github.com/miopunch/miopunch/internal/control"
	"github.com/miopunch/miopunch/internal/punching"
	"github.com/miopunch/miopunch/internal/wire"
)

type ClientConfig struct {
	CoordAddr    string
	ControlProto control.Protocol

	Signaling string // coord | mqtt

	MQTTBroker      string
	MQTTTopicPrefix string
	MQTTUser        string
	MQTTPass        string

	User       string
	ProxyName  string
	SecretKey  string
	AllowUsers []string

	DataProto string // kcp | quic
	QuicCC    string // bbr | brutal (only when DataProto=quic)

	DisableAuth bool

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
	P2PNetwork            connectivity.P2PNetwork
	DisablePortMap        bool
	P2PListenAddr         string
	DisableAssistedAddrs  bool
	HelloTimeout          time.Duration
	ExchangeInfoTimeout   time.Duration
	SessionOverallTimeout time.Duration

	Once bool

	Emitter *event.Emitter
}

func RunClient(ctx context.Context, cfg ClientConfig) error {
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
	if !cfg.DisableAuth && strings.TrimSpace(cfg.SecretKey) == "" {
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
	family, err := connectivity.ParseP2PIPFamily(string(cfg.P2PIPFamily))
	if err != nil {
		return fail(event.StageSupervisor, err, "invalid config", map[string]any{"p2p_ip_family": cfg.P2PIPFamily})
	}
	cfg.P2PIPFamily = family

	network, err := connectivity.ParseP2PNetwork(string(cfg.P2PNetwork))
	if err != nil {
		return fail(event.StageSupervisor, err, "invalid config", map[string]any{"p2p_network": cfg.P2PNetwork})
	}
	cfg.P2PNetwork = network

	if cfg.Signaling == "mqtt" {
		return runClientMQTT(ctx, cfg)
	}

	sess, err := dialHello(ctx, cfg.CoordAddr, cfg.ControlProto, &wire.PeerHello{
		Role:         "client",
		User:         cfg.User,
		Capabilities: []string{wire.CapabilityTCPP2PV0},
		P2PNetwork:   string(cfg.P2PNetwork),
		ProxyName:    cfg.ProxyName,
		SecretKey:    cfg.SecretKey,
		AllowUsers:   cfg.AllowUsers,
		DisableAuth:  cfg.DisableAuth,
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

	sidCh := make(chan string, 16)
	sess.dispatcher.RegisterHandler(&wire.NatHoleSid{}, wire.AsyncHandler(func(m wire.Message) {
		in := m.(*wire.NatHoleSid)
		if strings.TrimSpace(in.Sid) == "" {
			return
		}
		select {
		case sidCh <- in.Sid:
		default:
		}
	}))

	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-sess.dispatcher.Done():
			return nil
		case sid := <-sidCh:
			err := runClientSession(ctx, sess, cfg, sid)
			if err != nil && cfg.Emitter != nil {
				cfg.Emitter.Fail(event.StageSupervisor, err, "client session failed", map[string]any{
					"sid": sid,
				})
			}
			if cfg.Once {
				return err
			}
		}
	}
}

func runClientSession(ctx context.Context, sess *controlSession, cfg ClientConfig, sid string) error {
	sessionCtx, cancel := context.WithTimeout(ctx, cfg.SessionOverallTimeout)
	defer cancel()

	listenPort, err := listenPortFromAddr(cfg.P2PListenAddr)
	if err != nil {
		return err
	}
	gather, err := connectivity.Gather(sessionCtx, sid, connectivity.GatherConfig{
		ListenPort:           listenPort,
		P2PNetwork:           cfg.P2PNetwork,
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
			cfg.Emitter.Fail(event.StageGather, err, "gather failed", map[string]any{"sid": sid})
		}
		return err
	}
	if gather.UDP4Conn != nil {
		defer gather.UDP4Conn.Close()
	}
	if gather.UDP6Conn != nil {
		defer gather.UDP6Conn.Close()
	}
	if gather.TCP4Listener != nil {
		defer gather.TCP4Listener.Close()
	}
	if gather.TCP6Listener != nil {
		defer gather.TCP6Listener.Close()
	}

	transactionID := punching.NewTransactionID()

	// P3(v1): brutal parameters are intentionally fixed for lab regression.
	var brutalUpBps, brutalDownBps uint64
	if cfg.DataProto == "quic" && cfg.QuicCC == "brutal" {
		brutalUpBps = 1_000_000
		brutalDownBps = 1_000_000
	}

	natHoleClientMsg := &wire.NatHoleClient{
		TransactionID: transactionID,
		ProxyName:     cfg.ProxyName,
		Sid:           sid,
		Protocol:      cfg.DataProto,
		QuicCC:        cfg.QuicCC,
		BrutalUpBps:   brutalUpBps,
		BrutalDownBps: brutalDownBps,
		Capabilities:  []string{wire.CapabilityTCPP2PV0},
		P2PNetwork:    string(cfg.P2PNetwork),
		DirectAddrs:   gather.DirectAddrs,
		MappedAddrs:   gather.MappedAddrs,
		AssistedAddrs: gather.AssistedAddrs,
		TCPDirectAddrs: gather.TCPDirectAddrs,
		TCPMappedAddrs: gather.TCPMappedAddrs,
		TCPSTUNCN:      gather.TCPSTUNCN,
		TCPSTUNGlobal:  gather.TCPSTUNGlobal,
		STUNCN:        gather.STUNCN,
		STUNGlobal:    gather.STUNGlobal,
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(event.StageExchange, "exchange.start", map[string]any{
			"sid":           sid,
			"tx":            transactionID,
			"control_proto": string(cfg.ControlProto),
			"data_proto":    cfg.DataProto,
			"quic_cc":       cfg.QuicCC,
		})
	}

	natHoleRespMsg, err := punching.ExchangeInfo(sessionCtx, sess.xport, transactionID, natHoleClientMsg, cfg.ExchangeInfoTimeout)
	if err != nil {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(event.StageExchange, err, "exchange.failed", map[string]any{
				"sid": sid,
				"tx":  transactionID,
			})
		}
		return err
	}

	if cfg.Emitter != nil {
		cfg.Emitter.OK(event.StageExchange, "exchange.ok", map[string]any{
			"sid":             sid,
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

		attemptRes, err := connectivity.Attempt(sessionCtx, sid, []byte(cfg.SecretKey), gather.UDP4Conn, gather.UDP6Conn, gather.TCP4Listener, gather.TCP6Listener, natHoleRespMsg, connectivity.AttemptConfig{
			AttemptV6Timeout:      cfg.AttemptV6Timeout,
			AttemptPortmapTimeout: cfg.AttemptPortmapTimeout,
			P2PNetwork:            cfg.P2PNetwork,
			P2PIPFamily:           cfg.P2PIPFamily,
			Emitter:               cfg.Emitter,
	})
	if err != nil {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(event.StageAttempt, err, "attempt failed", map[string]any{"sid": sid})
		}
		return err
	}

	if attemptRes.Path == "punching_ipv4" {
		_ = sess.xport.Send(&wire.NatHoleReport{Sid: natHoleRespMsg.Sid, Success: true})
	}

		if cfg.Emitter != nil {
			dataProto := natHoleRespMsg.Protocol
			quicCC := natHoleRespMsg.QuicCC
			if len(attemptRes.TCPConns) > 0 {
				dataProto = string(dataplane.ProtocolTLS)
				quicCC = ""
			}
			cfg.Emitter.Start(event.StageTransport, "data plane start", map[string]any{
				"sid":        sid,
				"data_proto": dataProto,
				"quic_cc":    quicCC,
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
		dataProto := natHoleRespMsg.Protocol
		if len(attemptRes.TCPConns) > 0 {
			dataProto = string(dataplane.ProtocolTLS)
			dpCfg.Proto = dataplane.ProtocolTLS
		}

		dataErr := error(nil)
		if len(attemptRes.TCPConns) > 0 {
			dataErr = dataplane.ServeAndExchangeTLS(sessionCtx, dpCfg, natHoleRespMsg.Sid, []byte(cfg.SecretKey), attemptRes.TCPConns, cfg.Emitter)
		} else {
			dataErr = dataplane.ServeAndExchange(sessionCtx, dpCfg, attemptRes.Conn, attemptRes.Remote, cfg.Emitter)
		}
		if dataErr != nil && cfg.Emitter != nil {
			cfg.Emitter.Fail(event.StageTransport, dataErr, "data plane failed", map[string]any{
				"sid":        sid,
				"data_proto": dataProto,
			})
		}
		return dataErr
	}
