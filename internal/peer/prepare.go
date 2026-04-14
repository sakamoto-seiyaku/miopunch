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
