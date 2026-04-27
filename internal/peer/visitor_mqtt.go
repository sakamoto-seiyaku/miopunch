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
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/event"

	"github.com/miopunch/miopunch/internal/punchdecision"
	"github.com/miopunch/miopunch/internal/punching"
	mqttsig "github.com/miopunch/miopunch/internal/signaling/mqtt"
	"github.com/miopunch/miopunch/internal/wire"
)

func runVisitorMQTT(ctx context.Context, cfg VisitorConfig) error {
	fail := func(stage event.Stage, err error, msg string, kvs map[string]any) error {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(stage, err, msg, kvs)
		}
		return err
	}

	sessionCtx, cancel := context.WithTimeout(ctx, cfg.SessionOverallTimeout)
	defer cancel()

	sid := mqttsig.DeriveSID(cfg.ProxyName, cfg.SecretKey)

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
		return fail(event.StageGather, err, "gather failed", map[string]any{"sid": sid})
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

	natHoleVisitorMsg := &wire.NatHoleVisitor{
		TransactionID:  transactionID,
		ProxyName:      cfg.ProxyName,
		Protocol:       cfg.DataProto,
		QuicCC:         cfg.QuicCC,
		BrutalUpBps:    brutalUpBps,
		BrutalDownBps:  brutalDownBps,
		Capabilities:   []string{wire.CapabilityTCPP2PV0},
		P2PNetwork:     string(cfg.P2PNetwork),
		DirectAddrs:    gather.DirectAddrs,
		MappedAddrs:    gather.MappedAddrs,
		AssistedAddrs:  gather.AssistedAddrs,
		TCPDirectAddrs: gather.TCPDirectAddrs,
		TCPMappedAddrs: gather.TCPMappedAddrs,
		TCPSTUNCN:      gather.TCPSTUNCN,
		TCPSTUNGlobal:  gather.TCPSTUNGlobal,
		STUNCN:         gather.STUNCN,
		STUNGlobal:     gather.STUNGlobal,
	}

	brokerURL, err := buildMQTTBrokerURL(cfg.MQTTBroker, cfg.MQTTUser, cfg.MQTTPass)
	if err != nil {
		return fail(event.StageSupervisor, err, "invalid mqtt config", nil)
	}
	brokerURLForLog := mqttBrokerURLForLog(brokerURL)
	brokerURL, err = resolveMQTTBrokerURL(sessionCtx, brokerURL, cfg.BuiltinDNSMode, cfg.BuiltinDNSServers)
	if err != nil {
		return fail(event.StageSignaling, err, "mqtt broker resolve failed", map[string]any{"broker": brokerURLForLog, "sid": sid})
	}
	brokerURLForLog = mqttBrokerURLForLog(brokerURL)

	if cfg.Emitter != nil {
		cfg.Emitter.Start(event.StageSignaling, "mqtt connect start", map[string]any{
			"broker": brokerURLForLog,
			"sid":    sid,
		})
	}
	mq, err := mqttsig.Open(sessionCtx, mqttsig.Config{
		BrokerURL:       brokerURL,
		TopicPrefix:     cfg.MQTTTopicPrefix,
		SID:             sid,
		Role:            mqttsig.RoleVisitor,
		HelloTimeout:    cfg.HelloTimeout,
		ExchangeTimeout: cfg.ExchangeInfoTimeout,
		BarrierTimeout:  cfg.ExchangeInfoTimeout,
		StartDelay:      500 * time.Millisecond,
	})
	if err != nil {
		return fail(event.StageSignaling, err, "mqtt connect failed", map[string]any{"broker": brokerURLForLog, "sid": sid})
	}
	defer mq.Close()
	if cfg.Emitter != nil {
		cfg.Emitter.OK(event.StageSignaling, "connected to mqtt broker", map[string]any{
			"broker": brokerURLForLog,
			"sid":    sid,
		})
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(event.StageExchange, "exchange.start", map[string]any{
			"sid":        sid,
			"tx":         transactionID,
			"signaling":  "mqtt",
			"data_proto": cfg.DataProto,
			"quic_cc":    cfg.QuicCC,
		})
	}

	var decisionRes *punchdecision.Result
	natHoleRespMsg, err := mq.RunVisitor(sessionCtx, natHoleVisitorMsg, func(sid string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*wire.NatHoleResp, *wire.NatHoleResp, error) {
		res, err := punchdecision.AnalyzeWithDaemonMemory(sid, "", visitor, client)
		if err != nil {
			return nil, nil, err
		}
		decisionRes = res
		return res.VisitorResponse, res.ClientResponse, nil
	})
	if err != nil {
		return fail(event.StageExchange, err, "exchange.failed", map[string]any{"sid": sid, "tx": transactionID})
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

	attemptRes, err := connectivity.Attempt(sessionCtx, natHoleRespMsg.Sid, []byte(cfg.SecretKey), gather.UDP4Conn, gather.UDP6Conn, gather.TCP4Listener, gather.TCP6Listener, natHoleRespMsg, connectivity.AttemptConfig{
		AttemptV6Timeout:      cfg.AttemptV6Timeout,
		AttemptPortmapTimeout: cfg.AttemptPortmapTimeout,
		P2PNetwork:            cfg.P2PNetwork,
		P2PIPFamily:           cfg.P2PIPFamily,
		Emitter:               cfg.Emitter,
	})
	if err != nil {
		return fail(event.StageAttempt, err, "attempt failed", map[string]any{"sid": natHoleRespMsg.Sid})
	}
	switch attemptRes.Path {
	case "punching_ipv4":
		punchdecision.ReportDaemonUDPSuccess(decisionRes)
	case "punching_tcp4":
		punchdecision.ReportDaemonTCPSuccess(decisionRes)
	}

	if cfg.Emitter != nil {
		dataProto := natHoleRespMsg.Protocol
		quicCC := natHoleRespMsg.QuicCC
		if len(attemptRes.TCPConns) > 0 {
			dataProto = string(dataplane.ProtocolTLS)
			quicCC = ""
		}
		cfg.Emitter.Start(event.StageTransport, "data plane start", map[string]any{
			"sid":        natHoleRespMsg.Sid,
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
		dataErr = dataplane.DialAndExchangeTLS(sessionCtx, dpCfg, natHoleRespMsg.Sid, []byte(cfg.SecretKey), attemptRes.TCPConns, cfg.Payload, cfg.Emitter)
	} else {
		dataErr = dataplane.DialAndExchange(sessionCtx, dpCfg, attemptRes.Conn, attemptRes.Remote, cfg.Payload, cfg.Emitter)
	}
	if dataErr != nil && cfg.Emitter != nil {
		cfg.Emitter.Fail(event.StageTransport, dataErr, "data plane failed", map[string]any{
			"sid":        natHoleRespMsg.Sid,
			"data_proto": dataProto,
		})
	}
	return dataErr
}
