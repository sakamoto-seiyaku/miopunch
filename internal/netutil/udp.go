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

import "net"

// ConnectedUDPConn is a wrapper for net.UDPConn which converts WriteTo syscalls
// to Write syscalls that are faster on some OS'es.
// This should only be used for connections that were produced by a net.Dial* call.
type ConnectedUDPConn struct{ *net.UDPConn }

// WriteTo redirects all writes to the Write syscall.
func (c *ConnectedUDPConn) WriteTo(b []byte, _ net.Addr) (int, error) { return c.Write(b) }
