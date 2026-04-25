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

package main

import (
	"context"
	"flag"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/control"
	"github.com/miopunch/miopunch/internal/coordinator"
	"github.com/miopunch/miopunch/internal/peer"

	"github.com/miopunch/miopunch/stun"
)

func main() {
	os.Exit(run(os.Args[1:], os.Stdout, os.Stderr))
}

func run(args []string, stdout, stderr io.Writer) int {
	if len(args) < 1 {
		usage(stderr)
		return 2
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	switch args[0] {
	case "coord":
		return coordCmd(ctx, args[1:], stdout, stderr)
	case "mqtt-broker":
		return mqttBrokerCmd(ctx, args[1:], stdout, stderr)
	case "peer":
		return peerCmd(ctx, args[1:], stdout, stderr)
	case "stun":
		return stunCmd(ctx, args[1:], stdout, stderr)
	case "-h", "--help", "help":
		usage(stdout)
		return 0
	default:
		fmt.Fprintf(stderr, "unknown command: %s\n\n", args[0])
		usage(stderr)
		return 2
	}
}

func usage(w io.Writer) {
	fmt.Fprintf(w, `miopunch-lab (lab toolchain)

Usage:
  miopunch-lab coord  [flags]
  miopunch-lab mqtt-broker [flags]
  miopunch-lab peer   <client|visitor> [flags]
  miopunch-lab stun   [flags]
  miopunch-lab stun probe [flags]

Commands:
  coord:
    --listen <ip:port>
    --proto  <tcp|kcp|quic>   (control plane protocol)

	  peer client:
	    --config <path.yaml>
	    --log-level <trace|debug|info|warn|error>
	    --signaling <coord|mqtt>
	    --coord <ip:port>
	    --control-proto <tcp|kcp|quic>
	    --mqtt-broker <host:port|tcp://...>
	    --mqtt-topic-prefix <prefix>
	    --mqtt-user <user>
	    --mqtt-pass <pass>
	    --proxy <name>
	    --secret <secret>
	    --user <name>
	    --allow-users <comma-list>
	    --data-proto <kcp|quic>
	    --quic-cc <bbr|brutal>
	    -4                          (p2p/punching ipv4 only)
	    -6                          (p2p/punching ipv6 only)
	    --p2p-port <port>           (lab/test only; fixed local UDP port; 0=random)
	    --stun <addr1,addr2,...>
	    --stun-timeout <duration>
	    --gather-timeout <duration>           (portmap cutoff; STUN gating uses --stun-timeout)
	    --attempt-v6-timeout <duration>
	    --attempt-portmap-timeout <duration>
	    --disable-portmap
	    --once

	  peer visitor:
	    --config <path.yaml>
	    --log-level <trace|debug|info|warn|error>
	    --signaling <coord|mqtt>
	    --coord <ip:port>
	    --control-proto <tcp|kcp|quic>
	    --mqtt-broker <host:port|tcp://...>
	    --mqtt-topic-prefix <prefix>
	    --mqtt-user <user>
	    --mqtt-pass <pass>
	    --proxy <name>
	    --secret <secret>
	    --user <name>
	    --data-proto <kcp|quic>
	    --quic-cc <bbr|brutal>
	    --payload <string>
	    -4                          (p2p/punching ipv4 only)
	    -6                          (p2p/punching ipv6 only)
	    --p2p-port <port>           (lab/test only; fixed local UDP port; 0=random)
	    --stun <addr1,addr2,...>
	    --stun-timeout <duration>
	    --gather-timeout <duration>           (portmap cutoff; STUN gating uses --stun-timeout)
	    --attempt-v6-timeout <duration>
	    --attempt-portmap-timeout <duration>
	    --disable-portmap

  mqtt-broker:
    --listen <ip:port>   (default: 0.0.0.0:1883)
    --log-level <trace|debug|info|warn|error>

  stun:
    --listen <ip:port>   (repeatable)
    --log-level <trace|debug|info|warn|error>

  stun probe:
    --builtin | --stun <addr1,addr2,...>
    --attempts <n>           (default: 3)
    --ok-threshold <n>       (default: 2)
    --timeout <duration>     (default: 2s)
    --dial-timeout <duration> (default: 2s)
    --concurrency <n>        (default: 8)
    --builtin-dns-mode <auto|on|off>
    --builtin-dns <ip[:port],...>
    --out <path>
`)
}

func coordCmd(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	fs := flag.NewFlagSet("coord", flag.ContinueOnError)
	fs.SetOutput(stderr)
	listen := fs.String("listen", "127.0.0.1:7000", "listen address")
	proto := fs.String("proto", "tcp", "control plane protocol: tcp|kcp|quic")
	reserve := fs.Duration("reserve", 24*time.Hour, "analysis reserve duration")
	helloTimeout := fs.Duration("hello-timeout", 5*time.Second, "hello timeout")
	logLevel := addLogLevelFlag(fs)
	if err := fs.Parse(args); err != nil {
		if err == flag.ErrHelp {
			return 0
		}
		return 2
	}
	applyLogLevel(*logLevel)

	em := event.NewEmitter(stdout, "coord")
	cfg := coordinator.Config{
		ListenAddr:              *listen,
		Protocol:                control.Protocol(*proto),
		AnalysisReserveDuration: *reserve,
		HelloTimeout:            *helloTimeout,
		Emitter:                 em,
	}
	if err := coordinator.Run(ctx, cfg); err != nil {
		fmt.Fprintln(stderr, err)
		return 1
	}
	return 0
}

func peerCmd(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	if len(args) < 1 {
		usage(stderr)
		return 2
	}

	switch args[0] {
	case "client":
		return peerClientCmd(ctx, args[1:], stdout, stderr)
	case "visitor":
		return peerVisitorCmd(ctx, args[1:], stdout, stderr)
	default:
		fmt.Fprintf(stderr, "unknown peer role: %s\n\n", args[0])
		usage(stderr)
		return 2
	}
}

func peerClientCmd(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	fs := flag.NewFlagSet("peer client", flag.ContinueOnError)
	fs.SetOutput(stderr)
	configFile := fs.String("config", "", "config file (yaml)")
	logLevel := addLogLevelFlag(fs)
	signaling := fs.String("signaling", "coord", "signaling backend: coord|mqtt")
	coordAddr := fs.String("coord", "127.0.0.1:7000", "coordinator address")
	controlProto := fs.String("control-proto", "tcp", "control plane protocol: tcp|kcp|quic")
	proxy := fs.String("proxy", "", "proxy name")
	secret := fs.String("secret", "", "secret key")
	user := fs.String("user", "client", "user name")
	allowUsers := fs.String("allow-users", "", "comma-separated allow users (empty -> same user only)")
	mqttBroker := fs.String("mqtt-broker", "", "mqtt broker address (host:port or tcp://...)")
	mqttTopicPrefix := fs.String("mqtt-topic-prefix", "miopunch/p3.5", "mqtt topic prefix")
	mqttUser := fs.String("mqtt-user", "", "mqtt username")
	mqttPass := fs.String("mqtt-pass", "", "mqtt password")
	dataProto := fs.String("data-proto", "quic", "data plane protocol: kcp|quic")
	quicCC := fs.String("quic-cc", "bbr", "quic congestion control: bbr|brutal (only applies when --data-proto=quic)")
	p2pV4Only := fs.Bool("4", false, "p2p/punching ipv4 only")
	p2pV6Only := fs.Bool("6", false, "p2p/punching ipv6 only")
	p2pNetwork := fs.String("p2p-network", "auto", "p2p network policy: auto|udp_only|tcp_only")
	p2pUDPOnly := fs.Bool("u", false, "p2p network policy: udp_only")
	p2pTCPOnly := fs.Bool("t", false, "p2p network policy: tcp_only")
	builtinDNSMode := fs.String("builtin-dns-mode", "auto", "built-in dns mode for STUN/MQTT resolution: auto|on|off")
	builtinDNS := fs.String("builtin-dns", "", "comma-separated built-in dns resolvers (ip[:port]) for STUN/MQTT resolution (TCP/53)")
	stunServers := fs.String("stun", "", "comma-separated stun servers")
	stunTimeout := fs.Duration("stun-timeout", 3*time.Second, "STUN timeout (only applies when STUN servers are configured)")
	gatherTimeout := fs.Duration("gather-timeout", 1500*time.Millisecond, "gather timeout for optional helpers (e.g. portmap); does not gate STUN")
	attemptV6Timeout := fs.Duration("attempt-v6-timeout", 800*time.Millisecond, "attempt timeout for IPv6 direct")
	attemptPortmapTimeout := fs.Duration("attempt-portmap-timeout", 800*time.Millisecond, "attempt timeout for IPv4 direct (portmap)")
	disablePortmap := fs.Bool("disable-portmap", false, "disable IPv4 port mapping helpers")
	p2pPort := fs.Int("p2p-port", 0, "lab/test only: fixed local UDP port for NAT traversal (0=random)")
	once := fs.Bool("once", false, "exit after handling one session")
	disableAssisted := fs.Bool("disable-assisted", false, "disable assisted addrs")
	helloTimeout := fs.Duration("hello-timeout", 5*time.Second, "hello timeout")
	exchangeTimeout := fs.Duration("exchange-timeout", 5*time.Second, "exchangeInfo timeout")
	overallTimeout := fs.Duration("overall-timeout", 60*time.Second, "per-session overall timeout")
	if err := fs.Parse(args); err != nil {
		if err == flag.ErrHelp {
			return 0
		}
		return 2
	}
	applyLogLevel(*logLevel)

	set := map[string]bool{}
	fs.Visit(func(f *flag.Flag) { set[f.Name] = true })
	var p2pIPFamilyFromYAML *string
	stunExplicit := set["stun"]
	if strings.TrimSpace(*configFile) != "" {
		ycfg, err := loadPeerYAMLConfig(*configFile)
		if err != nil {
			fmt.Fprintln(stderr, err)
			return 1
		}
		p2pIPFamilyFromYAML = ycfg.P2PIPFamily
		if ycfg.P2PNetwork != nil && !set["p2p-network"] && !set["u"] && !set["t"] {
			*p2pNetwork = *ycfg.P2PNetwork
		}
		if ycfg.BuiltinDNSMode != nil && !set["builtin-dns-mode"] {
			*builtinDNSMode = *ycfg.BuiltinDNSMode
		}
		if len(ycfg.BuiltinDNS) > 0 && !set["builtin-dns"] {
			*builtinDNS = strings.Join(ycfg.BuiltinDNS, ",")
		}
		if ycfg.Signaling != nil && !set["signaling"] {
			*signaling = *ycfg.Signaling
		}
		if ycfg.Coord != nil && !set["coord"] {
			*coordAddr = *ycfg.Coord
		}
		if ycfg.ControlProto != nil && !set["control-proto"] {
			*controlProto = *ycfg.ControlProto
		}
		if ycfg.Proxy != nil && !set["proxy"] {
			*proxy = *ycfg.Proxy
		}
		if ycfg.Secret != nil && !set["secret"] {
			*secret = *ycfg.Secret
		}
		if ycfg.User != nil && !set["user"] {
			*user = *ycfg.User
		}
		if len(ycfg.AllowUsers) > 0 && !set["allow-users"] {
			*allowUsers = strings.Join(ycfg.AllowUsers, ",")
		}
		if ycfg.DataProto != nil && !set["data-proto"] {
			*dataProto = *ycfg.DataProto
		}
		if ycfg.QuicCC != nil && !set["quic-cc"] {
			*quicCC = *ycfg.QuicCC
		}
		if ycfg.P2PPort != nil && !set["p2p-port"] {
			*p2pPort = *ycfg.P2PPort
		}
		if ycfg.Stun != nil {
			stunExplicit = true
		}
		if ycfg.Stun != nil && !set["stun"] {
			*stunServers = strings.Join(ycfg.Stun, ",")
		}
		if ycfg.StunTimeout != nil && !set["stun-timeout"] {
			*stunTimeout = ycfg.StunTimeout.Duration
		}
		if ycfg.GatherTimeout != nil && !set["gather-timeout"] {
			*gatherTimeout = ycfg.GatherTimeout.Duration
		}
		if ycfg.AttemptV6Timeout != nil && !set["attempt-v6-timeout"] {
			*attemptV6Timeout = ycfg.AttemptV6Timeout.Duration
		}
		if ycfg.AttemptPortmapTimeout != nil && !set["attempt-portmap-timeout"] {
			*attemptPortmapTimeout = ycfg.AttemptPortmapTimeout.Duration
		}
		if ycfg.DisablePortmap != nil && !set["disable-portmap"] {
			*disablePortmap = *ycfg.DisablePortmap
		}
		if ycfg.DisableAssisted != nil && !set["disable-assisted"] {
			*disableAssisted = *ycfg.DisableAssisted
		}
		if ycfg.HelloTimeout != nil && !set["hello-timeout"] {
			*helloTimeout = ycfg.HelloTimeout.Duration
		}
		if ycfg.ExchangeTimeout != nil && !set["exchange-timeout"] {
			*exchangeTimeout = ycfg.ExchangeTimeout.Duration
		}
		if ycfg.OverallTimeout != nil && !set["overall-timeout"] {
			*overallTimeout = ycfg.OverallTimeout.Duration
		}
		if ycfg.Once != nil && !set["once"] {
			*once = *ycfg.Once
		}
		if ycfg.MQTTBroker != nil && !set["mqtt-broker"] {
			*mqttBroker = *ycfg.MQTTBroker
		}
		if ycfg.MQTTTopicPrefix != nil && !set["mqtt-topic-prefix"] {
			*mqttTopicPrefix = *ycfg.MQTTTopicPrefix
		}
		if ycfg.MQTTUser != nil && !set["mqtt-user"] {
			*mqttUser = *ycfg.MQTTUser
		}
		if ycfg.MQTTPass != nil && !set["mqtt-pass"] {
			*mqttPass = *ycfg.MQTTPass
		}
	}

	if *p2pV4Only && *p2pV6Only {
		fmt.Fprintln(stderr, "invalid config: -4 and -6 are mutually exclusive")
		return 2
	}

	p2pIPFamily := connectivity.P2PIPFamilyAuto
	if p2pIPFamilyFromYAML != nil && !set["4"] && !set["6"] {
		family, err := connectivity.ParseP2PIPFamily(*p2pIPFamilyFromYAML)
		if err != nil {
			fmt.Fprintln(stderr, err)
			return 2
		}
		p2pIPFamily = family
	}
	if *p2pV4Only {
		p2pIPFamily = connectivity.P2PIPFamilyV4
	}
	if *p2pV6Only {
		p2pIPFamily = connectivity.P2PIPFamilyV6
	}

	if *p2pUDPOnly && *p2pTCPOnly {
		fmt.Fprintln(stderr, "invalid p2p network: -u and -t are mutually exclusive")
		return 2
	}
	p2pNetworkValue := strings.TrimSpace(*p2pNetwork)
	if *p2pUDPOnly {
		p2pNetworkValue = "udp_only"
	}
	if *p2pTCPOnly {
		p2pNetworkValue = "tcp_only"
	}
	parsedNetwork, err := connectivity.ParseP2PNetwork(p2pNetworkValue)
	if err != nil {
		fmt.Fprintln(stderr, err)
		return 2
	}

	em := event.NewEmitter(stdout, "peer-client")
	cfg := peer.ClientConfig{
		CoordAddr:             *coordAddr,
		ControlProto:          control.Protocol(*controlProto),
		Signaling:             *signaling,
		MQTTBroker:            *mqttBroker,
		MQTTTopicPrefix:       *mqttTopicPrefix,
		MQTTUser:              *mqttUser,
		MQTTPass:              *mqttPass,
		User:                  *user,
		ProxyName:             *proxy,
		SecretKey:             *secret,
		AllowUsers:            splitComma(*allowUsers),
		DataProto:             *dataProto,
		QuicCC:                *quicCC,
		StunServers:           splitComma(*stunServers),
		StunExplicit:          stunExplicit,
		BuiltinDNSMode:        *builtinDNSMode,
		BuiltinDNSServers:     splitComma(*builtinDNS),
		StunTimeout:           *stunTimeout,
		GatherTimeout:         *gatherTimeout,
		AttemptV6Timeout:      *attemptV6Timeout,
		AttemptPortmapTimeout: *attemptPortmapTimeout,
		P2PIPFamily:           p2pIPFamily,
		P2PNetwork:            parsedNetwork,
		DisablePortMap:        *disablePortmap,
		DisableAssistedAddrs:  *disableAssisted,
		HelloTimeout:          *helloTimeout,
		ExchangeInfoTimeout:   *exchangeTimeout,
		SessionOverallTimeout: *overallTimeout,
		Once:                  *once,
		Emitter:               em,
	}
	if *p2pPort > 0 {
		cfg.P2PListenAddr = fmt.Sprintf(":%d", *p2pPort)
	}
	if err := peer.RunClient(ctx, cfg); err != nil {
		fmt.Fprintln(stderr, err)
		return 1
	}
	return 0
}

func peerVisitorCmd(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	fs := flag.NewFlagSet("peer visitor", flag.ContinueOnError)
	fs.SetOutput(stderr)
	configFile := fs.String("config", "", "config file (yaml)")
	logLevel := addLogLevelFlag(fs)
	signaling := fs.String("signaling", "coord", "signaling backend: coord|mqtt")
	coordAddr := fs.String("coord", "127.0.0.1:7000", "coordinator address")
	controlProto := fs.String("control-proto", "tcp", "control plane protocol: tcp|kcp|quic")
	proxy := fs.String("proxy", "", "proxy name")
	secret := fs.String("secret", "", "secret key")
	user := fs.String("user", "visitor", "user name")
	mqttBroker := fs.String("mqtt-broker", "", "mqtt broker address (host:port or tcp://...)")
	mqttTopicPrefix := fs.String("mqtt-topic-prefix", "miopunch/p3.5", "mqtt topic prefix")
	mqttUser := fs.String("mqtt-user", "", "mqtt username")
	mqttPass := fs.String("mqtt-pass", "", "mqtt password")
	dataProto := fs.String("data-proto", "quic", "data plane protocol: kcp|quic")
	quicCC := fs.String("quic-cc", "bbr", "quic congestion control: bbr|brutal (only applies when --data-proto=quic)")
	p2pV4Only := fs.Bool("4", false, "p2p/punching ipv4 only")
	p2pV6Only := fs.Bool("6", false, "p2p/punching ipv6 only")
	p2pNetwork := fs.String("p2p-network", "auto", "p2p network policy: auto|udp_only|tcp_only")
	p2pUDPOnly := fs.Bool("u", false, "p2p network policy: udp_only")
	p2pTCPOnly := fs.Bool("t", false, "p2p network policy: tcp_only")
	builtinDNSMode := fs.String("builtin-dns-mode", "auto", "built-in dns mode for STUN/MQTT resolution: auto|on|off")
	builtinDNS := fs.String("builtin-dns", "", "comma-separated built-in dns resolvers (ip[:port]) for STUN/MQTT resolution (TCP/53)")
	payload := fs.String("payload", "ping", "payload to send")
	stunServers := fs.String("stun", "", "comma-separated stun servers")
	stunTimeout := fs.Duration("stun-timeout", 3*time.Second, "STUN timeout (only applies when STUN servers are configured)")
	gatherTimeout := fs.Duration("gather-timeout", 1500*time.Millisecond, "gather timeout for optional helpers (e.g. portmap); does not gate STUN")
	attemptV6Timeout := fs.Duration("attempt-v6-timeout", 800*time.Millisecond, "attempt timeout for IPv6 direct")
	attemptPortmapTimeout := fs.Duration("attempt-portmap-timeout", 800*time.Millisecond, "attempt timeout for IPv4 direct (portmap)")
	disablePortmap := fs.Bool("disable-portmap", false, "disable IPv4 port mapping helpers")
	p2pPort := fs.Int("p2p-port", 0, "lab/test only: fixed local UDP port for NAT traversal (0=random)")
	disableAssisted := fs.Bool("disable-assisted", false, "disable assisted addrs")
	helloTimeout := fs.Duration("hello-timeout", 5*time.Second, "hello timeout")
	exchangeTimeout := fs.Duration("exchange-timeout", 5*time.Second, "exchangeInfo timeout")
	overallTimeout := fs.Duration("overall-timeout", 60*time.Second, "per-session overall timeout")
	if err := fs.Parse(args); err != nil {
		if err == flag.ErrHelp {
			return 0
		}
		return 2
	}
	applyLogLevel(*logLevel)

	set := map[string]bool{}
	fs.Visit(func(f *flag.Flag) { set[f.Name] = true })
	var p2pIPFamilyFromYAML *string
	stunExplicit := set["stun"]
	if strings.TrimSpace(*configFile) != "" {
		ycfg, err := loadPeerYAMLConfig(*configFile)
		if err != nil {
			fmt.Fprintln(stderr, err)
			return 1
		}
		p2pIPFamilyFromYAML = ycfg.P2PIPFamily
		if ycfg.P2PNetwork != nil && !set["p2p-network"] && !set["u"] && !set["t"] {
			*p2pNetwork = *ycfg.P2PNetwork
		}
		if ycfg.BuiltinDNSMode != nil && !set["builtin-dns-mode"] {
			*builtinDNSMode = *ycfg.BuiltinDNSMode
		}
		if len(ycfg.BuiltinDNS) > 0 && !set["builtin-dns"] {
			*builtinDNS = strings.Join(ycfg.BuiltinDNS, ",")
		}
		if ycfg.Signaling != nil && !set["signaling"] {
			*signaling = *ycfg.Signaling
		}
		if ycfg.Coord != nil && !set["coord"] {
			*coordAddr = *ycfg.Coord
		}
		if ycfg.ControlProto != nil && !set["control-proto"] {
			*controlProto = *ycfg.ControlProto
		}
		if ycfg.Proxy != nil && !set["proxy"] {
			*proxy = *ycfg.Proxy
		}
		if ycfg.Secret != nil && !set["secret"] {
			*secret = *ycfg.Secret
		}
		if ycfg.User != nil && !set["user"] {
			*user = *ycfg.User
		}
		if ycfg.DataProto != nil && !set["data-proto"] {
			*dataProto = *ycfg.DataProto
		}
		if ycfg.QuicCC != nil && !set["quic-cc"] {
			*quicCC = *ycfg.QuicCC
		}
		if ycfg.Payload != nil && !set["payload"] {
			*payload = *ycfg.Payload
		}
		if ycfg.P2PPort != nil && !set["p2p-port"] {
			*p2pPort = *ycfg.P2PPort
		}
		if ycfg.Stun != nil {
			stunExplicit = true
		}
		if ycfg.Stun != nil && !set["stun"] {
			*stunServers = strings.Join(ycfg.Stun, ",")
		}
		if ycfg.StunTimeout != nil && !set["stun-timeout"] {
			*stunTimeout = ycfg.StunTimeout.Duration
		}
		if ycfg.GatherTimeout != nil && !set["gather-timeout"] {
			*gatherTimeout = ycfg.GatherTimeout.Duration
		}
		if ycfg.AttemptV6Timeout != nil && !set["attempt-v6-timeout"] {
			*attemptV6Timeout = ycfg.AttemptV6Timeout.Duration
		}
		if ycfg.AttemptPortmapTimeout != nil && !set["attempt-portmap-timeout"] {
			*attemptPortmapTimeout = ycfg.AttemptPortmapTimeout.Duration
		}
		if ycfg.DisablePortmap != nil && !set["disable-portmap"] {
			*disablePortmap = *ycfg.DisablePortmap
		}
		if ycfg.DisableAssisted != nil && !set["disable-assisted"] {
			*disableAssisted = *ycfg.DisableAssisted
		}
		if ycfg.HelloTimeout != nil && !set["hello-timeout"] {
			*helloTimeout = ycfg.HelloTimeout.Duration
		}
		if ycfg.ExchangeTimeout != nil && !set["exchange-timeout"] {
			*exchangeTimeout = ycfg.ExchangeTimeout.Duration
		}
		if ycfg.OverallTimeout != nil && !set["overall-timeout"] {
			*overallTimeout = ycfg.OverallTimeout.Duration
		}
		if ycfg.MQTTBroker != nil && !set["mqtt-broker"] {
			*mqttBroker = *ycfg.MQTTBroker
		}
		if ycfg.MQTTTopicPrefix != nil && !set["mqtt-topic-prefix"] {
			*mqttTopicPrefix = *ycfg.MQTTTopicPrefix
		}
		if ycfg.MQTTUser != nil && !set["mqtt-user"] {
			*mqttUser = *ycfg.MQTTUser
		}
		if ycfg.MQTTPass != nil && !set["mqtt-pass"] {
			*mqttPass = *ycfg.MQTTPass
		}
	}

	if *p2pV4Only && *p2pV6Only {
		fmt.Fprintln(stderr, "invalid config: -4 and -6 are mutually exclusive")
		return 2
	}

	p2pIPFamily := connectivity.P2PIPFamilyAuto
	if p2pIPFamilyFromYAML != nil && !set["4"] && !set["6"] {
		family, err := connectivity.ParseP2PIPFamily(*p2pIPFamilyFromYAML)
		if err != nil {
			fmt.Fprintln(stderr, err)
			return 2
		}
		p2pIPFamily = family
	}
	if *p2pV4Only {
		p2pIPFamily = connectivity.P2PIPFamilyV4
	}
	if *p2pV6Only {
		p2pIPFamily = connectivity.P2PIPFamilyV6
	}

	if *p2pUDPOnly && *p2pTCPOnly {
		fmt.Fprintln(stderr, "invalid p2p network: -u and -t are mutually exclusive")
		return 2
	}
	p2pNetworkValue := strings.TrimSpace(*p2pNetwork)
	if *p2pUDPOnly {
		p2pNetworkValue = "udp_only"
	}
	if *p2pTCPOnly {
		p2pNetworkValue = "tcp_only"
	}
	parsedNetwork, err := connectivity.ParseP2PNetwork(p2pNetworkValue)
	if err != nil {
		fmt.Fprintln(stderr, err)
		return 2
	}

	em := event.NewEmitter(stdout, "peer-visitor")
	cfg := peer.VisitorConfig{
		CoordAddr:             *coordAddr,
		ControlProto:          control.Protocol(*controlProto),
		Signaling:             *signaling,
		MQTTBroker:            *mqttBroker,
		MQTTTopicPrefix:       *mqttTopicPrefix,
		MQTTUser:              *mqttUser,
		MQTTPass:              *mqttPass,
		User:                  *user,
		ProxyName:             *proxy,
		SecretKey:             *secret,
		DataProto:             *dataProto,
		QuicCC:                *quicCC,
		Payload:               []byte(*payload),
		StunServers:           splitComma(*stunServers),
		StunExplicit:          stunExplicit,
		BuiltinDNSMode:        *builtinDNSMode,
		BuiltinDNSServers:     splitComma(*builtinDNS),
		StunTimeout:           *stunTimeout,
		GatherTimeout:         *gatherTimeout,
		AttemptV6Timeout:      *attemptV6Timeout,
		AttemptPortmapTimeout: *attemptPortmapTimeout,
		P2PIPFamily:           p2pIPFamily,
		P2PNetwork:            parsedNetwork,
		DisablePortMap:        *disablePortmap,
		DisableAssistedAddrs:  *disableAssisted,
		HelloTimeout:          *helloTimeout,
		ExchangeInfoTimeout:   *exchangeTimeout,
		SessionOverallTimeout: *overallTimeout,
		Emitter:               em,
	}
	if *p2pPort > 0 {
		cfg.P2PListenAddr = fmt.Sprintf(":%d", *p2pPort)
	}
	if err := peer.RunVisitor(ctx, cfg); err != nil {
		fmt.Fprintln(stderr, err)
		return 1
	}
	return 0
}

func stunCmd(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	if len(args) > 0 && args[0] == "probe" {
		return stunProbeCmd(ctx, args[1:], stdout, stderr)
	}

	_ = stdout
	fs := flag.NewFlagSet("stun", flag.ContinueOnError)
	fs.SetOutput(stderr)
	var listens multiString
	fs.Var(&listens, "listen", "listen address (repeatable)")
	logLevel := addLogLevelFlag(fs)
	if err := fs.Parse(args); err != nil {
		if err == flag.ErrHelp {
			return 0
		}
		return 2
	}
	applyLogLevel(*logLevel)

	if len(listens) == 0 {
		listens = append(listens, "0.0.0.0:3478", "0.0.0.0:3479")
	}

	s, err := stun.ListenAndServe(ctx, listens)
	if err != nil {
		fmt.Fprintln(stderr, err)
		return 1
	}
	<-ctx.Done()
	s.Close()
	return 0
}

type multiString []string

func (m *multiString) String() string { return strings.Join(*m, ",") }
func (m *multiString) Set(s string) error {
	*m = append(*m, s)
	return nil
}

func splitComma(s string) []string {
	out := make([]string, 0)
	for _, part := range strings.Split(s, ",") {
		part = strings.TrimSpace(part)
		if part == "" {
			continue
		}
		out = append(out, part)
	}
	return out
}
