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

package coordinator

import (
	"context"
	"errors"
	"fmt"
	"io"
	"strings"
	"time"

	"github.com/miopunch/miopunch/event"

	"github.com/miopunch/miopunch/internal/control"
	"github.com/miopunch/miopunch/internal/wire"
)

type Config struct {
	ListenAddr string
	Protocol   control.Protocol

	AnalysisReserveDuration time.Duration
	HelloTimeout            time.Duration

	Emitter *event.Emitter
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
		cfg.Emitter.OK(event.StageSupervisor, "coordinator listening", map[string]any{
			"addr":  cfg.ListenAddr,
			"proto": string(cfg.Protocol),
		})
	}

	ctrl, err := NewController(cfg.AnalysisReserveDuration)
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

func handleConn(ctx context.Context, rwc io.ReadWriteCloser, ctrl *Controller, cfg Config) {
	defer rwc.Close()

	disp := wire.NewDispatcher(rwc)
	xport := wire.NewMessageTransporter(disp)

	helloCh := make(chan *wire.PeerHello, 1)
	disp.RegisterHandler(&wire.PeerHello{}, func(m wire.Message) {
		select {
		case helloCh <- m.(*wire.PeerHello):
		default:
		}
	})

	disp.Run()

	var hello *wire.PeerHello
	select {
	case <-ctx.Done():
		return
	case <-disp.Done():
		return
	case <-time.After(cfg.HelloTimeout):
		_ = disp.Send(&wire.PeerHelloResp{Error: "hello timeout"})
		return
	case hello = <-helloCh:
	}

	role := strings.TrimSpace(hello.Role)
	user := strings.TrimSpace(hello.User)
	switch role {
	case "client":
		proxyName := strings.TrimSpace(hello.ProxyName)
		if proxyName == "" {
			_ = disp.Send(&wire.PeerHelloResp{Error: "client proxy_name is required"})
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
			_ = disp.Send(&wire.PeerHelloResp{Error: "client secret_key is required"})
			return
		}

		sidCh, err := ctrl.ListenClient(proxyName, hello.SecretKey, allowUsers)
		if err != nil {
			_ = disp.Send(&wire.PeerHelloResp{Error: err.Error()})
			return
		}

		if cfg.Emitter != nil {
			cfg.Emitter.OK(event.StageSignaling, "client registered", map[string]any{
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
					_ = disp.Send(&wire.NatHoleSid{Sid: sid})
				}
			}
		}()
		go func() {
			<-disp.Done()
			close(stopSid)
			ctrl.CloseClient(proxyName)
		}()

		disp.RegisterHandler(&wire.NatHoleClient{}, func(m wire.Message) {
			in := m.(*wire.NatHoleClient)
			ctrl.HandleClient(in, xport)
		})
		disp.RegisterHandler(&wire.NatHoleReport{}, func(m wire.Message) {
			ctrl.HandleReport(m.(*wire.NatHoleReport))
		})

		_ = disp.Send(&wire.PeerHelloResp{})
		<-disp.Done()
		return

	case "visitor":
		if user == "" {
			user = "visitor"
		}

		if cfg.Emitter != nil {
			cfg.Emitter.OK(event.StageSignaling, "visitor connected", map[string]any{
				"user": user,
			})
		}

		disp.RegisterHandler(&wire.NatHoleVisitor{}, wire.AsyncHandler(func(m wire.Message) {
			in := m.(*wire.NatHoleVisitor)
			ctrl.HandleVisitor(in, xport, user)
		}))
		disp.RegisterHandler(&wire.NatHoleReport{}, func(m wire.Message) {
			ctrl.HandleReport(m.(*wire.NatHoleReport))
		})

		_ = disp.Send(&wire.PeerHelloResp{})
		<-disp.Done()
		return

	default:
		_ = disp.Send(&wire.PeerHelloResp{Error: fmt.Sprintf("unknown role: %q", role)})
		return
	}
}
