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

package msg

import (
	"net"
	"reflect"
)

// NOTE: Type bytes for NAT-hole messages intentionally match frp's values, to
// make copy-first extraction and diffing easier.
const (
	TypePeerHello     = 'a'
	TypePeerHelloResp = 'b'

	TypeNatHoleVisitor = 'i'
	TypeNatHoleClient  = 'n'
	TypeNatHoleResp    = 'm'
	TypeNatHoleSid     = '5'
	TypeNatHoleReport  = '6'
)

var msgTypeMap = map[byte]any{
	TypePeerHello:     PeerHello{},
	TypePeerHelloResp: PeerHelloResp{},

	TypeNatHoleVisitor: NatHoleVisitor{},
	TypeNatHoleClient:  NatHoleClient{},
	TypeNatHoleResp:    NatHoleResp{},
	TypeNatHoleSid:     NatHoleSid{},
	TypeNatHoleReport:  NatHoleReport{},
}

var TypeNameNatHoleResp = reflect.TypeFor[NatHoleResp]().Name()

type PeerHello struct {
	Role string `json:"role,omitempty"` // client | visitor
	User string `json:"user,omitempty"`

	// client only
	ProxyName   string   `json:"proxy_name,omitempty"`
	SecretKey   string   `json:"secret_key,omitempty"`
	AllowUsers  []string `json:"allow_users,omitempty"`
	DisableAuth bool     `json:"disable_auth,omitempty"` // test-only escape hatch
}

type PeerHelloResp struct {
	Error string `json:"error,omitempty"`
}

type NatHoleVisitor struct {
	TransactionID string   `json:"transaction_id,omitempty"`
	ProxyName     string   `json:"proxy_name,omitempty"`
	PreCheck      bool     `json:"pre_check,omitempty"`
	Protocol      string   `json:"protocol,omitempty"` // kcp | quic (data plane)
	SignKey       string   `json:"sign_key,omitempty"`
	Timestamp     int64    `json:"timestamp,omitempty"`
	MappedAddrs   []string `json:"mapped_addrs,omitempty"`
	AssistedAddrs []string `json:"assisted_addrs,omitempty"`
}

type NatHoleClient struct {
	TransactionID string   `json:"transaction_id,omitempty"`
	ProxyName     string   `json:"proxy_name,omitempty"`
	Sid           string   `json:"sid,omitempty"`
	MappedAddrs   []string `json:"mapped_addrs,omitempty"`
	AssistedAddrs []string `json:"assisted_addrs,omitempty"`
}

type PortsRange struct {
	From int `json:"from,omitempty"`
	To   int `json:"to,omitempty"`
}

type NatHoleDetectBehavior struct {
	Role              string       `json:"role,omitempty"` // sender or receiver
	Mode              int          `json:"mode,omitempty"` // 0, 1, 2...
	TTL               int          `json:"ttl,omitempty"`
	SendDelayMs       int          `json:"send_delay_ms,omitempty"`
	ReadTimeoutMs     int          `json:"read_timeout,omitempty"`
	CandidatePorts    []PortsRange `json:"candidate_ports,omitempty"`
	SendRandomPorts   int          `json:"send_random_ports,omitempty"`
	ListenRandomPorts int          `json:"listen_random_ports,omitempty"`
}

type NatHoleResp struct {
	TransactionID  string                `json:"transaction_id,omitempty"`
	Sid            string                `json:"sid,omitempty"`
	Protocol       string                `json:"protocol,omitempty"`
	CandidateAddrs []string              `json:"candidate_addrs,omitempty"`
	AssistedAddrs  []string              `json:"assisted_addrs,omitempty"`
	DetectBehavior NatHoleDetectBehavior `json:"detect_behavior,omitempty"`
	Error          string                `json:"error,omitempty"`
}

// NatHoleSid is used in two places:
// 1) control plane: coordinator -> client, notifying a new session sid
// 2) punching: exchanged on UDP after encryption/auth (see xtcp/nathole)
type NatHoleSid struct {
	TransactionID string `json:"transaction_id,omitempty"`
	Sid           string `json:"sid,omitempty"`
	Response      bool   `json:"response,omitempty"`
	Nonce         string `json:"nonce,omitempty"`
}

type NatHoleReport struct {
	Sid     string `json:"sid,omitempty"`
	Success bool   `json:"success,omitempty"`
}

type UDPPacket struct {
	Content    []byte       `json:"c,omitempty"`
	LocalAddr  *net.UDPAddr `json:"l,omitempty"`
	RemoteAddr *net.UDPAddr `json:"r,omitempty"`
}
