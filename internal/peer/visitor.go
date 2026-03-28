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
	"net"
	"strings"
	"time"

	"github.com/quic-go/quic-go"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/event"

	"github.com/miopunch/miopunch/internal/authutil"
	"github.com/miopunch/miopunch/internal/control"
	"github.com/miopunch/miopunch/internal/netutil"
	"github.com/miopunch/miopunch/internal/punching"
	"github.com/miopunch/miopunch/internal/tlsutil"
	"github.com/miopunch/miopunch/internal/wire"
)

type VisitorConfig struct {
	CoordAddr    string
	ControlProto control.Protocol

	User      string
	ProxyName string
	SecretKey string
	DataProto string // kcp | quic

	Payload []byte

	StunServers           []string
	StunTimeout           time.Duration
	GatherTimeout         time.Duration
	AttemptV6Timeout      time.Duration
	AttemptPortmapTimeout time.Duration
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
		DisableAssistedAddrs: cfg.DisableAssistedAddrs,
		DisablePortMap:       cfg.DisablePortMap,
		StunServers:          cfg.StunServers,
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
	defer gather.UDP4Conn.Close()
	if gather.UDP6Conn != nil {
		defer gather.UDP6Conn.Close()
	}

	now := time.Now().Unix()
	transactionID := punching.NewTransactionID()
	natHoleVisitorMsg := &wire.NatHoleVisitor{
		TransactionID: transactionID,
		ProxyName:     cfg.ProxyName,
		Protocol:      cfg.DataProto,
		SignKey:       authutil.GetAuthKey(cfg.SecretKey, now),
		Timestamp:     now,
		DirectAddrs:   gather.DirectAddrs,
		MappedAddrs:   gather.MappedAddrs,
		AssistedAddrs: gather.AssistedAddrs,
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(event.StageExchange, "exchange.start", map[string]any{
			"tx":            transactionID,
			"control_proto": string(cfg.ControlProto),
			"data_proto":    cfg.DataProto,
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
			"sid":         natHoleRespMsg.Sid,
			"tx":          transactionID,
			"data_proto":  natHoleRespMsg.Protocol,
			"peer_direct": len(natHoleRespMsg.PeerDirectAddrs),
			"punching":    natHoleRespMsg.PunchingEnabled,
		})
	}

	attemptRes, err := connectivity.Attempt(sessionCtx, natHoleRespMsg.Sid, []byte(cfg.SecretKey), gather.UDP4Conn, gather.UDP6Conn, natHoleRespMsg, connectivity.AttemptConfig{
		AttemptV6Timeout:      cfg.AttemptV6Timeout,
		AttemptPortmapTimeout: cfg.AttemptPortmapTimeout,
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
		})
	}

	var dataErr error
	switch natHoleRespMsg.Protocol {
	case "kcp":
		dataErr = dialKCP(sessionCtx, cfg, attemptRes.Conn, attemptRes.Remote)
	case "quic", "":
		dataErr = dialQUIC(sessionCtx, cfg, attemptRes.Conn, attemptRes.Remote)
	default:
		dataErr = fmt.Errorf("unknown data plane protocol: %q", natHoleRespMsg.Protocol)
	}
	if dataErr != nil && cfg.Emitter != nil {
		cfg.Emitter.Fail(event.StageTransport, dataErr, "data plane failed", map[string]any{
			"sid":        natHoleRespMsg.Sid,
			"data_proto": natHoleRespMsg.Protocol,
		})
	}
	return dataErr
}

func dialKCP(ctx context.Context, cfg VisitorConfig, listenConn *net.UDPConn, raddr *net.UDPAddr) error {
	defer listenConn.Close()
	laddr, err := net.ResolveUDPAddr("udp", listenConn.LocalAddr().String())
	if err != nil {
		return err
	}
	_ = listenConn.Close()

	lConn, err := net.DialUDP("udp", laddr, raddr)
	if err != nil {
		return err
	}
	defer lConn.Close()

	c, err := netutil.NewKCPConnFromUDP(lConn, true, raddr.String())
	if err != nil {
		return err
	}
	defer c.Close()

	_ = c.SetDeadline(time.Now().Add(15 * time.Second))
	if err := writeFrame(c, cfg.Payload); err != nil {
		return err
	}
	resp, err := readFrame(c, 64*1024)
	if err != nil {
		return err
	}
	if string(resp) != "ok:"+string(cfg.Payload) {
		return fmt.Errorf("unexpected response: %q", string(resp))
	}

	if cfg.Emitter != nil {
		cfg.Emitter.OK(event.StageTransport, "kcp payload exchanged", map[string]any{
			"bytes": len(cfg.Payload),
		})
	}
	return nil
}

func dialQUIC(ctx context.Context, cfg VisitorConfig, listenConn *net.UDPConn, raddr *net.UDPAddr) error {
	defer listenConn.Close()

	tlsConfig, err := tlsutil.NewClientTLSConfig("", "", "", raddr.String())
	if err != nil {
		return err
	}
	tlsConfig.NextProtos = []string{"miopunch-xtcp-data"}

	c, err := quic.Dial(ctx, listenConn, raddr, tlsConfig, &quic.Config{
		HandshakeIdleTimeout: 20 * time.Second,
		MaxIdleTimeout:       30 * time.Second,
		KeepAlivePeriod:      10 * time.Second,
	})
	if err != nil {
		return err
	}
	defer c.CloseWithError(0, "")

	stream, err := c.OpenStreamSync(ctx)
	if err != nil {
		return err
	}
	defer stream.Close()

	_ = stream.SetDeadline(time.Now().Add(15 * time.Second))
	if err := writeFrame(stream, cfg.Payload); err != nil {
		return err
	}
	resp, err := readFrame(stream, 64*1024)
	if err != nil {
		return err
	}
	if string(resp) != "ok:"+string(cfg.Payload) {
		return fmt.Errorf("unexpected response: %q", string(resp))
	}

	if cfg.Emitter != nil {
		cfg.Emitter.OK(event.StageTransport, "quic payload exchanged", map[string]any{
			"bytes": len(cfg.Payload),
		})
	}
	return nil
}
