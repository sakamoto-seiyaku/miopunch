// Copyright 2017 fatedier, fatedier@gmail.com
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

package netutil

import (
	"net"

	kcp "github.com/xtaci/kcp-go/v5"
)

func NewKCPConnFromUDP(conn *net.UDPConn, connected bool, raddr string) (net.Conn, error) {
	udpAddr, err := net.ResolveUDPAddr("udp", raddr)
	if err != nil {
		return nil, err
	}
	var pConn net.PacketConn = conn
	if connected {
		pConn = &ConnectedUDPConn{conn}
	}
	// conv is part of the KCP header. For our current use (one session per peer
	// pair), a fixed conv keeps both ends aligned without needing extra
	// negotiation. Server-side multi-session support is handled by a listener
	// (ServeConn + AcceptKCP), not by varying conv here.
	kcpConn, err := kcp.NewConn3(1, udpAddr, nil, 10, 3, pConn)
	if err != nil {
		return nil, err
	}
	kcpConn.SetStreamMode(true)
	kcpConn.SetWriteDelay(true)
	kcpConn.SetNoDelay(1, 20, 2, 1)
	kcpConn.SetMtu(1350)
	kcpConn.SetWindowSize(1024, 1024)
	kcpConn.SetACKNoDelay(false)
	return kcpConn, nil
}
