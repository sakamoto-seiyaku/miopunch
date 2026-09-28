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
	"encoding/binary"
	"errors"
	"io"
	"net"
	"strconv"
	"sync"
	"time"

	"github.com/pion/stun/v2"
)

const maxTCPMessageSize = 2048

type Server struct {
	udpConns []*net.UDPConn
	tcpLns   []*net.TCPListener

	wg sync.WaitGroup
}

func ListenAndServe(ctx context.Context, addrs []string) (*Server, error) {
	if len(addrs) == 0 {
		return nil, errors.New("no listen addresses")
	}

	s := &Server{}
	for _, addr := range addrs {
		host, portText, err := net.SplitHostPort(addr)
		if err != nil {
			s.Close()
			return nil, err
		}
		port, err := strconv.Atoi(portText)
		if err != nil {
			s.Close()
			return nil, err
		}
		if port < 0 || port > 65535 {
			s.Close()
			return nil, errors.New("invalid port")
		}

		// Ensure TCP and UDP share the same port even when port=0.
		tcpAddr := addr
		udpAddr := addr
		if port == 0 {
			tmpLn, err := net.Listen("tcp", tcpAddr)
			if err != nil {
				s.Close()
				return nil, err
			}
			tl, ok := tmpLn.(*net.TCPListener)
			if !ok {
				_ = tmpLn.Close()
				s.Close()
				return nil, errors.New("listener is not TCP")
			}

			actualPort := tl.Addr().(*net.TCPAddr).Port
			udpAddr = net.JoinHostPort(host, strconv.Itoa(actualPort))
			ua, err := net.ResolveUDPAddr("udp", udpAddr)
			if err != nil {
				_ = tl.Close()
				s.Close()
				return nil, err
			}
			uc, err := net.ListenUDP("udp", ua)
			if err != nil {
				_ = tl.Close()
				s.Close()
				return nil, err
			}

			s.tcpLns = append(s.tcpLns, tl)
			s.udpConns = append(s.udpConns, uc)
			continue
		}

		ua, err := net.ResolveUDPAddr("udp", udpAddr)
		if err != nil {
			s.Close()
			return nil, err
		}
		uc, err := net.ListenUDP("udp", ua)
		if err != nil {
			s.Close()
			return nil, err
		}

		tmpLn, err := net.Listen("tcp", tcpAddr)
		if err != nil {
			_ = uc.Close()
			s.Close()
			return nil, err
		}
		tl, ok := tmpLn.(*net.TCPListener)
		if !ok {
			_ = tmpLn.Close()
			_ = uc.Close()
			s.Close()
			return nil, errors.New("listener is not TCP")
		}

		s.udpConns = append(s.udpConns, uc)
		s.tcpLns = append(s.tcpLns, tl)
	}

	for _, c := range s.udpConns {
		s.wg.Add(1)
		go func(conn *net.UDPConn) {
			defer s.wg.Done()
			serveUDPConn(ctx, conn)
		}(c)
	}

	for _, ln := range s.tcpLns {
		s.wg.Add(1)
		go func(ln *net.TCPListener) {
			defer s.wg.Done()
			serveTCPListener(ctx, ln)
		}(ln)
	}
	return s, nil
}

func (s *Server) Close() {
	for _, c := range s.udpConns {
		_ = c.Close()
	}
	for _, ln := range s.tcpLns {
		_ = ln.Close()
	}
	s.udpConns = nil
	s.tcpLns = nil
	s.wg.Wait()
}

func serveUDPConn(ctx context.Context, conn *net.UDPConn) {
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

func serveTCPListener(ctx context.Context, ln *net.TCPListener) {
	for {
		_ = ln.SetDeadline(time.Now().Add(500 * time.Millisecond))
		conn, err := ln.AcceptTCP()
		_ = ln.SetDeadline(time.Time{})
		if err != nil {
			var ne net.Error
			if errors.As(err, &ne) && ne.Timeout() {
				select {
				case <-ctx.Done():
					return
				default:
					continue
				}
			}
			return
		}

		go func(c *net.TCPConn) {
			defer c.Close()
			serveTCPConn(ctx, c)
		}(conn)
	}
}

func serveTCPConn(ctx context.Context, conn *net.TCPConn) {
	for {
		if deadline, ok := ctx.Deadline(); ok {
			_ = conn.SetDeadline(deadline)
		}

		header := make([]byte, 20)
		if _, err := io.ReadFull(conn, header); err != nil {
			return
		}

		msgLen := int(binary.BigEndian.Uint16(header[2:4]))
		if msgLen < 0 || msgLen > maxTCPMessageSize {
			return
		}

		body := make([]byte, msgLen)
		if _, err := io.ReadFull(conn, body); err != nil {
			return
		}

		raw := append(header, body...)

		var req stun.Message
		req.Raw = raw
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

		raddr, ok := conn.RemoteAddr().(*net.TCPAddr)
		if !ok {
			continue
		}
		xor := &stun.XORMappedAddress{
			IP:   raddr.IP,
			Port: raddr.Port,
		}
		_ = xor.AddTo(resp)
		if _, err := conn.Write(resp.Raw); err != nil {
			return
		}
	}
}
