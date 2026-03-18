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

	"github.com/miopunch/miopunch/xtcp/control"
	"github.com/miopunch/miopunch/xtcp/msg"
	"github.com/miopunch/miopunch/xtcp/nathole"
	"github.com/miopunch/miopunch/xtcp/netutil"
	"github.com/miopunch/miopunch/xtcp/obs"
	"github.com/miopunch/miopunch/xtcp/transport"
	"github.com/miopunch/miopunch/xtcp/util/util"
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
	P2PListenAddr         string
	DisableAssistedAddrs  bool
	HelloTimeout          time.Duration
	ExchangeInfoTimeout   time.Duration
	SessionOverallTimeout time.Duration

	Emitter *obs.Emitter
}

func RunVisitor(ctx context.Context, cfg VisitorConfig) error {
	fail := func(stage obs.Stage, err error, msg string, kvs map[string]any) error {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(stage, err, msg, kvs)
		}
		return err
	}

	if strings.TrimSpace(cfg.ProxyName) == "" {
		return fail(obs.StageSupervisor, errors.New("proxy name is required"), "invalid config", nil)
	}
	if strings.TrimSpace(cfg.SecretKey) == "" {
		return fail(obs.StageSupervisor, errors.New("secret key is required"), "invalid config", nil)
	}
	if cfg.DataProto == "" {
		cfg.DataProto = "quic"
	}
	if cfg.DataProto != "kcp" && cfg.DataProto != "quic" {
		return fail(obs.StageSupervisor, fmt.Errorf("unsupported data proto: %q", cfg.DataProto), "invalid config", nil)
	}
	if len(cfg.StunServers) == 0 {
		return fail(obs.StageSupervisor, errors.New("stun servers are required"), "invalid config", nil)
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

	sess, err := dialHello(ctx, cfg.CoordAddr, cfg.ControlProto, &msg.PeerHello{
		Role: "visitor",
		User: cfg.User,
	}, cfg.HelloTimeout)
	if err != nil {
		return fail(obs.StageSignaling, err, "connect to coordinator failed", map[string]any{
			"coord":         cfg.CoordAddr,
			"control_proto": string(cfg.ControlProto),
			"proxy_name":    cfg.ProxyName,
		})
	}
	defer sess.rwc.Close()

	if cfg.Emitter != nil {
		cfg.Emitter.OK(obs.StageSignaling, "connected to coordinator", map[string]any{
			"coord":         cfg.CoordAddr,
			"control_proto": string(cfg.ControlProto),
			"proxy_name":    cfg.ProxyName,
		})
	}

	sessionCtx, cancel := context.WithTimeout(ctx, cfg.SessionOverallTimeout)
	defer cancel()

	if cfg.Emitter != nil {
		cfg.Emitter.Start(obs.StageSignaling, "precheck start", map[string]any{
			"proxy_name": cfg.ProxyName,
		})
	}
	if err := nathole.PreCheck(sessionCtx, sess.xport, cfg.ProxyName, cfg.ExchangeInfoTimeout); err != nil {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(obs.StageSignaling, err, "precheck failed", map[string]any{
				"proxy_name": cfg.ProxyName,
			})
		}
		return err
	}
	if cfg.Emitter != nil {
		cfg.Emitter.OK(obs.StageSignaling, "precheck ok", nil)
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(obs.StageDiscovery, "prepare start", nil)
	}
	prepareResult, err := prepareNATHole(cfg.StunServers, cfg.DisableAssistedAddrs, cfg.P2PListenAddr)
	if err != nil {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(obs.StageDiscovery, err, "prepare failed", nil)
		}
		return err
	}
	defer prepareResult.ListenConn.Close()

	if cfg.Emitter != nil {
		cfg.Emitter.OK(obs.StageDiscovery, "prepare ok", map[string]any{
			"nat_type": prepareResult.NatType,
			"behavior": prepareResult.Behavior,
			"mapped":   prepareResult.Addrs,
		})
	}

	now := time.Now().Unix()
	transactionID := nathole.NewTransactionID()
	natHoleVisitorMsg := &msg.NatHoleVisitor{
		TransactionID: transactionID,
		ProxyName:     cfg.ProxyName,
		Protocol:      cfg.DataProto,
		SignKey:       util.GetAuthKey(cfg.SecretKey, now),
		Timestamp:     now,
		MappedAddrs:   prepareResult.Addrs,
		AssistedAddrs: prepareResult.AssistedAddrs,
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(obs.StageSignaling, "exchange info start", map[string]any{
			"tx":            transactionID,
			"control_proto": string(cfg.ControlProto),
			"data_proto":    cfg.DataProto,
		})
	}

	natHoleRespMsg, err := nathole.ExchangeInfo(sessionCtx, sess.xport, transactionID, natHoleVisitorMsg, cfg.ExchangeInfoTimeout)
	if err != nil {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(obs.StageSignaling, err, "exchange info failed", map[string]any{
				"tx": transactionID,
			})
		}
		return err
	}

	if cfg.Emitter != nil {
		cfg.Emitter.OK(obs.StageSignaling, "exchange info ok", map[string]any{
			"sid":        natHoleRespMsg.Sid,
			"tx":         transactionID,
			"data_proto": natHoleRespMsg.Protocol,
			"detect":     natHoleRespMsg.DetectBehavior,
		})
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(obs.StagePunching, "make hole start", map[string]any{
			"sid": natHoleRespMsg.Sid,
		})
	}

	listenConn := prepareResult.ListenConn
	newListenConn, raddr, err := nathole.MakeHole(sessionCtx, listenConn, natHoleRespMsg, []byte(cfg.SecretKey))
	if err != nil {
		listenConn.Close()
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(obs.StagePunching, err, "make hole failed", map[string]any{
				"sid": natHoleRespMsg.Sid,
			})
		}
		return err
	}
	listenConn = newListenConn

	if cfg.Emitter != nil {
		cfg.Emitter.OK(obs.StagePunching, "make hole ok", map[string]any{
			"sid":   natHoleRespMsg.Sid,
			"raddr": raddr.String(),
		})
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(obs.StageTransport, "data plane start", map[string]any{
			"sid":        natHoleRespMsg.Sid,
			"data_proto": natHoleRespMsg.Protocol,
		})
	}

	var dataErr error
	switch natHoleRespMsg.Protocol {
	case "kcp":
		dataErr = dialKCP(sessionCtx, cfg, listenConn, raddr)
	case "quic", "":
		dataErr = dialQUIC(sessionCtx, cfg, listenConn, raddr)
	default:
		dataErr = fmt.Errorf("unknown data plane protocol: %q", natHoleRespMsg.Protocol)
	}
	if dataErr != nil && cfg.Emitter != nil {
		cfg.Emitter.Fail(obs.StageTransport, dataErr, "data plane failed", map[string]any{
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
		cfg.Emitter.OK(obs.StageTransport, "kcp payload exchanged", map[string]any{
			"bytes": len(cfg.Payload),
		})
	}
	return nil
}

func dialQUIC(ctx context.Context, cfg VisitorConfig, listenConn *net.UDPConn, raddr *net.UDPAddr) error {
	defer listenConn.Close()

	tlsConfig, err := transport.NewClientTLSConfig("", "", "", raddr.String())
	if err != nil {
		return err
	}
	tlsConfig.NextProtos = []string{"miopunch-xtcp-data"}

	c, err := quic.Dial(ctx, listenConn, raddr, tlsConfig, &quic.Config{
		HandshakeIdleTimeout: 20 * time.Second,
		MaxIdleTimeout:  30 * time.Second,
		KeepAlivePeriod: 10 * time.Second,
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
		cfg.Emitter.OK(obs.StageTransport, "quic payload exchanged", map[string]any{
			"bytes": len(cfg.Payload),
		})
	}
	return nil
}
