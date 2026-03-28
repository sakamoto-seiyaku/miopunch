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
	"fmt"
	"net"
	"strconv"
	"strings"

	"github.com/miopunch/miopunch/xtcp/nathole"
)

func listenPortFromAddr(addr string) (int, error) {
	addr = strings.TrimSpace(addr)
	if addr == "" {
		return 0, nil
	}
	_, portStr, err := net.SplitHostPort(addr)
	if err != nil {
		// Allow passing a bare port number for tests.
		portStr = addr
	}
	port, err := strconv.Atoi(portStr)
	if err != nil {
		return 0, fmt.Errorf("invalid listen addr %q: %w", addr, err)
	}
	if port < 0 || port > 65535 {
		return 0, fmt.Errorf("invalid port in listen addr %q: %d", addr, port)
	}
	return port, nil
}

func prepareNATHole(stunServers []string, disableAssistedAddrs bool, localAddr string) (*nathole.PrepareResult, error) {
	opts := nathole.PrepareOptions{
		DisableAssistedAddrs: disableAssistedAddrs,
	}
	if localAddr == "" {
		return nathole.Prepare(stunServers, opts)
	}

	// Avoid modifying the copy-first extracted nathole.Prepare logic: we keep upstream code intact
	// and only glue a fixed local port for the lab environment here.
	addrs, discoveredLocalAddr, err := nathole.Discover(stunServers, localAddr)
	if err != nil {
		return nil, fmt.Errorf("discover error: %v", err)
	}
	if len(addrs) < 2 {
		return nil, fmt.Errorf("discover error: not enough addresses")
	}

	localIPs, _ := nathole.ListLocalIPsForNatHole(10)
	natFeature, err := nathole.ClassifyNATFeature(addrs, localIPs)
	if err != nil {
		return nil, fmt.Errorf("classify nat feature error: %v", err)
	}

	laddr, err := net.ResolveUDPAddr("udp4", discoveredLocalAddr.String())
	if err != nil {
		return nil, fmt.Errorf("resolve local udp addr error: %v", err)
	}
	listenConn, err := net.ListenUDP("udp4", laddr)
	if err != nil {
		return nil, fmt.Errorf("listen local udp addr error: %v", err)
	}

	var assistedAddrs []string
	if !opts.DisableAssistedAddrs {
		assistedAddrs = make([]string, 0, len(localIPs))
		for _, ip := range localIPs {
			assistedAddrs = append(assistedAddrs, net.JoinHostPort(ip, strconv.Itoa(laddr.Port)))
		}
	}
	return &nathole.PrepareResult{
		Addrs:         addrs,
		AssistedAddrs: assistedAddrs,
		ListenConn:    listenConn,
		NatType:       natFeature.NatType,
		Behavior:      natFeature.Behavior,
	}, nil
}
