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

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/event"

	"github.com/miopunch/miopunch/internal/punching"
	mqttsig "github.com/miopunch/miopunch/internal/signaling/mqtt"
	"github.com/miopunch/miopunch/internal/wire"
)

func runClientMQTT(ctx context.Context, cfg ClientConfig) error {
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
		DisableAssistedAddrs: cfg.DisableAssistedAddrs,
		DisablePortMap:       cfg.DisablePortMap,
		StunServers:          cfg.StunServers,
		StunTimeout:          cfg.StunTimeout,
		GatherTimeout:        cfg.GatherTimeout,
		SessionLease:         connectivity.PortMapLease(cfg.SessionOverallTimeout),
		Emitter:              cfg.Emitter,
	})
	if err != nil {
		return fail(event.StageGather, err, "gather failed", map[string]any{"sid": sid})
	}
	defer gather.UDP4Conn.Close()
	if gather.UDP6Conn != nil {
		defer gather.UDP6Conn.Close()
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
		DirectAddrs:   gather.DirectAddrs,
		MappedAddrs:   gather.MappedAddrs,
		AssistedAddrs: gather.AssistedAddrs,
	}

	brokerURL, err := buildMQTTBrokerURL(cfg.MQTTBroker, cfg.MQTTUser, cfg.MQTTPass)
	if err != nil {
		return fail(event.StageSupervisor, err, "invalid mqtt config", nil)
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(event.StageSignaling, "mqtt connect start", map[string]any{
			"broker": brokerURL,
			"sid":    sid,
		})
	}
	mq, err := mqttsig.Open(sessionCtx, mqttsig.Config{
		BrokerURL:       brokerURL,
		TopicPrefix:     cfg.MQTTTopicPrefix,
		SID:             sid,
		Role:            mqttsig.RoleClient,
		HelloTimeout:    cfg.HelloTimeout,
		ExchangeTimeout: cfg.ExchangeInfoTimeout,
		BarrierTimeout:  cfg.ExchangeInfoTimeout,
	})
	if err != nil {
		return fail(event.StageSignaling, err, "mqtt connect failed", map[string]any{"broker": brokerURL, "sid": sid})
	}
	defer mq.Close()
	if cfg.Emitter != nil {
		cfg.Emitter.OK(event.StageSignaling, "connected to mqtt broker", map[string]any{
			"broker": brokerURL,
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

	natHoleRespMsg, err := mq.RunClient(sessionCtx, natHoleClientMsg)
	if err != nil {
		return fail(event.StageExchange, err, "exchange.failed", map[string]any{"sid": sid, "tx": transactionID})
	}

	if cfg.Emitter != nil {
		cfg.Emitter.OK(event.StageExchange, "exchange.ok", map[string]any{
			"sid":         sid,
			"tx":          transactionID,
			"data_proto":  natHoleRespMsg.Protocol,
			"quic_cc":     natHoleRespMsg.QuicCC,
			"brutal_up":   natHoleRespMsg.BrutalUpBps,
			"brutal_down": natHoleRespMsg.BrutalDownBps,
			"peer_direct": len(natHoleRespMsg.PeerDirectAddrs),
			"punching":    natHoleRespMsg.PunchingEnabled,
		})
	}

	attemptRes, err := connectivity.Attempt(sessionCtx, sid, []byte(cfg.SecretKey), gather.UDP4Conn, gather.UDP6Conn, natHoleRespMsg, connectivity.AttemptConfig{
		AttemptV6Timeout:      cfg.AttemptV6Timeout,
		AttemptPortmapTimeout: cfg.AttemptPortmapTimeout,
		Emitter:               cfg.Emitter,
	})
	if err != nil {
		return fail(event.StageAttempt, err, "attempt failed", map[string]any{"sid": sid})
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(event.StageTransport, "data plane start", map[string]any{
			"sid":        sid,
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
	dataErr := dataplane.ServeAndExchange(sessionCtx, dpCfg, attemptRes.Conn, attemptRes.Remote, cfg.Emitter)
	if dataErr != nil && cfg.Emitter != nil {
		cfg.Emitter.Fail(event.StageTransport, dataErr, "data plane failed", map[string]any{
			"sid":        sid,
			"data_proto": natHoleRespMsg.Protocol,
		})
	}
	if dataErr != nil {
		return dataErr
	}

	// In mqtt mode, we only run one session and return success.
	return nil
}
