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

package coord

import (
	"context"
	"errors"
	"fmt"
	"io"
	"strings"
	"time"

	"github.com/miopunch/miopunch/xtcp/control"
	"github.com/miopunch/miopunch/xtcp/msg"
	"github.com/miopunch/miopunch/xtcp/nathole"
	"github.com/miopunch/miopunch/xtcp/obs"
	"github.com/miopunch/miopunch/xtcp/transport"
)

type Config struct {
	ListenAddr string
	Protocol   control.Protocol

	AnalysisReserveDuration time.Duration
	HelloTimeout            time.Duration

	Emitter *obs.Emitter
}

func Run(ctx context.Context, cfg Config) error {
	if cfg.ListenAddr == "" {
		return errors.New("listen addr is required")
	}
	if cfg.Protocol == "" {
		cfg.Protocol = control.ProtoTCP
	}
	if cfg.AnalysisReserveDuration == 0 {
		cfg.AnalysisReserveDuration = 24 * time.Hour
	}
	if cfg.HelloTimeout == 0 {
		cfg.HelloTimeout = 5 * time.Second
	}

	l, err := control.Listen(cfg.ListenAddr, cfg.Protocol)
	if err != nil {
		return err
	}
	defer l.Close()

	if cfg.Emitter != nil {
		cfg.Emitter.OK(obs.StageSupervisor, "coordinator listening", map[string]any{
			"addr":  cfg.ListenAddr,
			"proto": string(cfg.Protocol),
		})
	}

	ctrl, err := nathole.NewController(cfg.AnalysisReserveDuration)
	if err != nil {
		return err
	}
	go ctrl.CleanWorker(ctx)

	for {
		rwc, err := l.Accept(ctx)
		if err != nil {
			if ctx.Err() != nil {
				return nil
			}
			return err
		}
		go handleConn(ctx, rwc, ctrl, cfg)
	}
}

func handleConn(ctx context.Context, rwc io.ReadWriteCloser, ctrl *nathole.Controller, cfg Config) {
	defer rwc.Close()

	disp := msg.NewDispatcher(rwc)
	xport := transport.NewMessageTransporter(disp)

	helloCh := make(chan *msg.PeerHello, 1)
	disp.RegisterHandler(&msg.PeerHello{}, func(m msg.Message) {
		select {
		case helloCh <- m.(*msg.PeerHello):
		default:
		}
	})

	disp.Run()

	var hello *msg.PeerHello
	select {
	case <-ctx.Done():
		return
	case <-disp.Done():
		return
	case <-time.After(cfg.HelloTimeout):
		_ = disp.Send(&msg.PeerHelloResp{Error: "hello timeout"})
		return
	case hello = <-helloCh:
	}

	role := strings.TrimSpace(hello.Role)
	user := strings.TrimSpace(hello.User)
	switch role {
	case "client":
		proxyName := strings.TrimSpace(hello.ProxyName)
		if proxyName == "" {
			_ = disp.Send(&msg.PeerHelloResp{Error: "client proxy_name is required"})
			return
		}
		if user == "" {
			user = "client"
		}

		allowUsers := hello.AllowUsers
		if len(allowUsers) == 0 {
			// Align with frp's intent: empty allowUsers means "same user only".
			allowUsers = []string{user}
		}

		if !hello.DisableAuth && strings.TrimSpace(hello.SecretKey) == "" {
			_ = disp.Send(&msg.PeerHelloResp{Error: "client secret_key is required"})
			return
		}

		sidCh, err := ctrl.ListenClient(proxyName, hello.SecretKey, allowUsers)
		if err != nil {
			_ = disp.Send(&msg.PeerHelloResp{Error: err.Error()})
			return
		}

		if cfg.Emitter != nil {
			cfg.Emitter.OK(obs.StageSignaling, "client registered", map[string]any{
				"user":       user,
				"proxy_name": proxyName,
			})
		}

		stopSid := make(chan struct{})
		go func() {
			for {
				select {
				case <-stopSid:
					return
				case <-disp.Done():
					return
				case sid := <-sidCh:
					_ = disp.Send(&msg.NatHoleSid{Sid: sid})
				}
			}
		}()
		go func() {
			<-disp.Done()
			close(stopSid)
			ctrl.CloseClient(proxyName)
		}()

		disp.RegisterHandler(&msg.NatHoleClient{}, func(m msg.Message) {
			in := m.(*msg.NatHoleClient)
			ctrl.HandleClient(in, xport)
		})
		disp.RegisterHandler(&msg.NatHoleReport{}, func(m msg.Message) {
			ctrl.HandleReport(m.(*msg.NatHoleReport))
		})

		_ = disp.Send(&msg.PeerHelloResp{})
		<-disp.Done()
		return

	case "visitor":
		if user == "" {
			user = "visitor"
		}

		if cfg.Emitter != nil {
			cfg.Emitter.OK(obs.StageSignaling, "visitor connected", map[string]any{
				"user": user,
			})
		}

		disp.RegisterHandler(&msg.NatHoleVisitor{}, msg.AsyncHandler(func(m msg.Message) {
			in := m.(*msg.NatHoleVisitor)
			ctrl.HandleVisitor(in, xport, user)
		}))
		disp.RegisterHandler(&msg.NatHoleReport{}, func(m msg.Message) {
			ctrl.HandleReport(m.(*msg.NatHoleReport))
		})

		_ = disp.Send(&msg.PeerHelloResp{})
		<-disp.Done()
		return

	default:
		_ = disp.Send(&msg.PeerHelloResp{Error: fmt.Sprintf("unknown role: %q", role)})
		return
	}
}
