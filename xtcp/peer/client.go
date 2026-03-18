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
)

type ClientConfig struct {
	CoordAddr    string
	ControlProto control.Protocol

	User       string
	ProxyName  string
	SecretKey  string
	AllowUsers []string

	DisableAuth bool

	StunServers           []string
	P2PListenAddr         string
	DisableAssistedAddrs  bool
	HelloTimeout          time.Duration
	ExchangeInfoTimeout   time.Duration
	SessionOverallTimeout time.Duration

	Once bool

	Emitter *obs.Emitter
}

func RunClient(ctx context.Context, cfg ClientConfig) error {
	fail := func(stage obs.Stage, err error, msg string, kvs map[string]any) error {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(stage, err, msg, kvs)
		}
		return err
	}

	if strings.TrimSpace(cfg.ProxyName) == "" {
		return fail(obs.StageSupervisor, errors.New("proxy name is required"), "invalid config", nil)
	}
	if !cfg.DisableAuth && strings.TrimSpace(cfg.SecretKey) == "" {
		return fail(obs.StageSupervisor, errors.New("secret key is required"), "invalid config", nil)
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

	sess, err := dialHello(ctx, cfg.CoordAddr, cfg.ControlProto, &msg.PeerHello{
		Role:        "client",
		User:        cfg.User,
		ProxyName:   cfg.ProxyName,
		SecretKey:   cfg.SecretKey,
		AllowUsers:  cfg.AllowUsers,
		DisableAuth: cfg.DisableAuth,
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

	sidCh := make(chan string, 16)
	sess.dispatcher.RegisterHandler(&msg.NatHoleSid{}, msg.AsyncHandler(func(m msg.Message) {
		in := m.(*msg.NatHoleSid)
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
				cfg.Emitter.Fail(obs.StageSupervisor, err, "client session failed", map[string]any{
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

	if cfg.Emitter != nil {
		cfg.Emitter.Start(obs.StageDiscovery, "prepare start", map[string]any{
			"sid": sid,
		})
	}

	prepareResult, err := prepareNATHole(cfg.StunServers, cfg.DisableAssistedAddrs, cfg.P2PListenAddr)
	if err != nil {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(obs.StageDiscovery, err, "prepare failed", map[string]any{"sid": sid})
		}
		return err
	}
	defer prepareResult.ListenConn.Close()

	if cfg.Emitter != nil {
		cfg.Emitter.OK(obs.StageDiscovery, "prepare ok", map[string]any{
			"sid":      sid,
			"nat_type": prepareResult.NatType,
			"behavior": prepareResult.Behavior,
			"mapped":   prepareResult.Addrs,
		})
	}

	transactionID := nathole.NewTransactionID()
	natHoleClientMsg := &msg.NatHoleClient{
		TransactionID: transactionID,
		ProxyName:     cfg.ProxyName,
		Sid:           sid,
		MappedAddrs:   prepareResult.Addrs,
		AssistedAddrs: prepareResult.AssistedAddrs,
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(obs.StageSignaling, "exchange info start", map[string]any{
			"sid":           sid,
			"tx":            transactionID,
			"control_proto": string(cfg.ControlProto),
		})
	}

	natHoleRespMsg, err := nathole.ExchangeInfo(sessionCtx, sess.xport, transactionID, natHoleClientMsg, cfg.ExchangeInfoTimeout)
	if err != nil {
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(obs.StageSignaling, err, "exchange info failed", map[string]any{
				"sid": sid,
				"tx":  transactionID,
			})
		}
		return err
	}

	if cfg.Emitter != nil {
		cfg.Emitter.OK(obs.StageSignaling, "exchange info ok", map[string]any{
			"sid":        sid,
			"tx":         transactionID,
			"data_proto": natHoleRespMsg.Protocol,
			"detect":     natHoleRespMsg.DetectBehavior,
		})
	}

	if cfg.Emitter != nil {
		cfg.Emitter.Start(obs.StagePunching, "make hole start", map[string]any{
			"sid": sid,
		})
	}

	listenConn := prepareResult.ListenConn
	newListenConn, raddr, err := nathole.MakeHole(sessionCtx, listenConn, natHoleRespMsg, []byte(cfg.SecretKey))
	if err != nil {
		listenConn.Close()
		_ = sess.xport.Send(&msg.NatHoleReport{Sid: natHoleRespMsg.Sid, Success: false})
		if cfg.Emitter != nil {
			cfg.Emitter.Fail(obs.StagePunching, err, "make hole failed", map[string]any{
				"sid": sid,
			})
		}
		return err
	}
	listenConn = newListenConn

	if cfg.Emitter != nil {
		cfg.Emitter.OK(obs.StagePunching, "make hole ok", map[string]any{
			"sid":   sid,
			"raddr": raddr.String(),
		})
	}

	_ = sess.xport.Send(&msg.NatHoleReport{Sid: natHoleRespMsg.Sid, Success: true})

	if cfg.Emitter != nil {
		cfg.Emitter.Start(obs.StageTransport, "data plane start", map[string]any{
			"sid":        sid,
			"data_proto": natHoleRespMsg.Protocol,
		})
	}

	var dataErr error
	switch natHoleRespMsg.Protocol {
	case "kcp":
		dataErr = serveKCP(sessionCtx, cfg, listenConn, raddr)
	case "quic", "":
		dataErr = serveQUIC(sessionCtx, cfg, listenConn)
	default:
		dataErr = fmt.Errorf("unknown data plane protocol: %q", natHoleRespMsg.Protocol)
	}
	if dataErr != nil && cfg.Emitter != nil {
		cfg.Emitter.Fail(obs.StageTransport, dataErr, "data plane failed", map[string]any{
			"sid":        sid,
			"data_proto": natHoleRespMsg.Protocol,
		})
	}
	return dataErr
}

func serveKCP(ctx context.Context, cfg ClientConfig, listenConn *net.UDPConn, raddr *net.UDPAddr) error {
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
	req, err := readFrame(c, 64*1024)
	if err != nil {
		return err
	}
	resp := append([]byte("ok:"), req...)
	if err := writeFrame(c, resp); err != nil {
		return err
	}

	if cfg.Emitter != nil {
		cfg.Emitter.OK(obs.StageTransport, "kcp payload exchanged", map[string]any{
			"bytes": len(req),
		})
	}

	// Keep the UDP socket alive briefly so the dialer side can finish its exchange
	// without receiving ICMP "port unreachable" due to early close.
	select {
	case <-ctx.Done():
	case <-time.After(750 * time.Millisecond):
	}
	return nil
}

func serveQUIC(ctx context.Context, cfg ClientConfig, listenConn *net.UDPConn) error {
	defer listenConn.Close()

	tlsConfig, err := transport.NewServerTLSConfig("", "", "")
	if err != nil {
		return err
	}
	tlsConfig.NextProtos = []string{"miopunch-xtcp-data"}

	quicListener, err := quic.Listen(listenConn, tlsConfig, &quic.Config{
		HandshakeIdleTimeout: 20 * time.Second,
		MaxIdleTimeout:  30 * time.Second,
		KeepAlivePeriod: 10 * time.Second,
	})
	if err != nil {
		return err
	}
	defer quicListener.Close()

	c, err := quicListener.Accept(ctx)
	if err != nil {
		return err
	}
	defer c.CloseWithError(0, "")

	stream, err := c.AcceptStream(ctx)
	if err != nil {
		return err
	}
	defer stream.Close()

	_ = stream.SetDeadline(time.Now().Add(15 * time.Second))
	req, err := readFrame(stream, 64*1024)
	if err != nil {
		return err
	}
	resp := append([]byte("ok:"), req...)
	if err := writeFrame(stream, resp); err != nil {
		return err
	}

	if cfg.Emitter != nil {
		cfg.Emitter.OK(obs.StageTransport, "quic payload exchanged", map[string]any{
			"bytes": len(req),
		})
	}

	// Wait for the dialer side to close the connection before closing the underlying UDP socket.
	// Closing too early may surface as "Application error 0x0 (remote)" on the visitor.
	select {
	case <-ctx.Done():
	case <-c.Context().Done():
	case <-time.After(2 * time.Second):
	}
	return nil
}
