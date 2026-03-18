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

package stun

import (
	"context"
	"errors"
	"net"
	"sync"
	"time"

	"github.com/pion/stun/v2"
)

type Server struct {
	conns []*net.UDPConn
	wg    sync.WaitGroup
}

func ListenAndServe(ctx context.Context, addrs []string) (*Server, error) {
	if len(addrs) == 0 {
		return nil, errors.New("no listen addresses")
	}

	s := &Server{}
	for _, addr := range addrs {
		udpAddr, err := net.ResolveUDPAddr("udp4", addr)
		if err != nil {
			s.Close()
			return nil, err
		}
		conn, err := net.ListenUDP("udp4", udpAddr)
		if err != nil {
			s.Close()
			return nil, err
		}
		s.conns = append(s.conns, conn)
	}

	for _, c := range s.conns {
		s.wg.Add(1)
		go func(conn *net.UDPConn) {
			defer s.wg.Done()
			serveConn(ctx, conn)
		}(c)
	}
	return s, nil
}

func (s *Server) Close() {
	for _, c := range s.conns {
		_ = c.Close()
	}
	s.conns = nil
	s.wg.Wait()
}

func serveConn(ctx context.Context, conn *net.UDPConn) {
	buf := make([]byte, 1500)
	for {
		_ = conn.SetReadDeadline(time.Now().Add(500 * time.Millisecond))
		n, raddr, err := conn.ReadFromUDP(buf)
		_ = conn.SetReadDeadline(time.Time{})
		if err != nil {
			if ne, ok := err.(net.Error); ok && ne.Timeout() {
				select {
				case <-ctx.Done():
					return
				default:
					continue
				}
			}
			return
		}

		var req stun.Message
		req.Raw = append([]byte(nil), buf[:n]...)
		if err := req.Decode(); err != nil {
			continue
		}
		if req.Type.Method != stun.MethodBinding || req.Type.Class != stun.ClassRequest {
			continue
		}

		resp, err := stun.Build(stun.NewTransactionIDSetter(req.TransactionID), stun.BindingSuccess)
		if err != nil {
			continue
		}

		xor := &stun.XORMappedAddress{
			IP:   raddr.IP,
			Port: raddr.Port,
		}
		_ = xor.AddTo(resp)
		_, _ = conn.WriteToUDP(resp.Raw, raddr)
	}
}
