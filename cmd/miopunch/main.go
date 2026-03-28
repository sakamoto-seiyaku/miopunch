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
	"os"
	"strings"
	"time"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/control"
	"github.com/miopunch/miopunch/internal/coordinator"
	"github.com/miopunch/miopunch/internal/peer"

	"github.com/miopunch/miopunch/stun"
)

func main() {
	if len(os.Args) < 2 {
		usage()
		os.Exit(2)
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	switch os.Args[1] {
	case "coord":
		coordCmd(ctx, os.Args[2:])
	case "peer":
		peerCmd(ctx, os.Args[2:])
	case "stun":
		stunCmd(ctx, os.Args[2:])
	case "-h", "--help", "help":
		usage()
	default:
		fmt.Fprintf(os.Stderr, "unknown command: %s\n", os.Args[1])
		usage()
		os.Exit(2)
	}
}

func usage() {
	fmt.Fprintf(os.Stderr, `miopunch (punching kernel)

Usage:
  miopunch coord  [flags]
  miopunch peer   <client|visitor> [flags]
  miopunch stun   [flags]

Commands:
  coord:
    --listen <ip:port>
    --proto  <tcp|kcp|quic>   (control plane protocol)

	  peer client:
	    --coord <ip:port>
	    --control-proto <tcp|kcp|quic>
	    --proxy <name>
	    --secret <secret>
	    --user <name>
	    --allow-users <comma-list>
	    --data-proto <kcp|quic>
	    --quic-cc <bbr|brutal>
	    --p2p-port <port>           (lab/test only; fixed local UDP port; 0=random)
	    --stun <addr1,addr2,...>
	    --stun-timeout <duration>
	    --gather-timeout <duration>           (portmap cutoff; STUN gating uses --stun-timeout)
	    --attempt-v6-timeout <duration>
	    --attempt-portmap-timeout <duration>
	    --disable-portmap
	    --once

	  peer visitor:
	    --coord <ip:port>
	    --control-proto <tcp|kcp|quic>
	    --proxy <name>
	    --secret <secret>
	    --user <name>
	    --data-proto <kcp|quic>
	    --quic-cc <bbr|brutal>
	    --payload <string>
	    --p2p-port <port>           (lab/test only; fixed local UDP port; 0=random)
	    --stun <addr1,addr2,...>
	    --stun-timeout <duration>
	    --gather-timeout <duration>           (portmap cutoff; STUN gating uses --stun-timeout)
	    --attempt-v6-timeout <duration>
	    --attempt-portmap-timeout <duration>
	    --disable-portmap

  stun:
    --listen <ip:port>   (repeatable)
`)
}

func coordCmd(ctx context.Context, args []string) {
	fs := flag.NewFlagSet("coord", flag.ExitOnError)
	listen := fs.String("listen", "127.0.0.1:7000", "listen address")
	proto := fs.String("proto", "tcp", "control plane protocol: tcp|kcp|quic")
	reserve := fs.Duration("reserve", 24*time.Hour, "analysis reserve duration")
	helloTimeout := fs.Duration("hello-timeout", 5*time.Second, "hello timeout")
	_ = fs.Parse(args)

	em := event.NewEmitter(os.Stdout, "coord")
	cfg := coordinator.Config{
		ListenAddr:              *listen,
		Protocol:                control.Protocol(*proto),
		AnalysisReserveDuration: *reserve,
		HelloTimeout:            *helloTimeout,
		Emitter:                 em,
	}
	if err := coordinator.Run(ctx, cfg); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func peerCmd(ctx context.Context, args []string) {
	if len(args) < 1 {
		usage()
		os.Exit(2)
	}

	switch args[0] {
	case "client":
		peerClientCmd(ctx, args[1:])
	case "visitor":
		peerVisitorCmd(ctx, args[1:])
	default:
		fmt.Fprintf(os.Stderr, "unknown peer role: %s\n", args[0])
		usage()
		os.Exit(2)
	}
}

func peerClientCmd(ctx context.Context, args []string) {
	fs := flag.NewFlagSet("peer client", flag.ExitOnError)
	coordAddr := fs.String("coord", "127.0.0.1:7000", "coordinator address")
	controlProto := fs.String("control-proto", "tcp", "control plane protocol: tcp|kcp|quic")
	proxy := fs.String("proxy", "", "proxy name")
	secret := fs.String("secret", "", "secret key")
	user := fs.String("user", "client", "user name")
	allowUsers := fs.String("allow-users", "", "comma-separated allow users (empty -> same user only)")
	dataProto := fs.String("data-proto", "quic", "data plane protocol: kcp|quic")
	quicCC := fs.String("quic-cc", "bbr", "quic congestion control: bbr|brutal (only applies when --data-proto=quic)")
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
	_ = fs.Parse(args)

	em := event.NewEmitter(os.Stdout, "peer-client")
	cfg := peer.ClientConfig{
		CoordAddr:             *coordAddr,
		ControlProto:          control.Protocol(*controlProto),
		User:                  *user,
		ProxyName:             *proxy,
		SecretKey:             *secret,
		AllowUsers:            splitComma(*allowUsers),
		DataProto:             *dataProto,
		QuicCC:                *quicCC,
		StunServers:           splitComma(*stunServers),
		StunTimeout:           *stunTimeout,
		GatherTimeout:         *gatherTimeout,
		AttemptV6Timeout:      *attemptV6Timeout,
		AttemptPortmapTimeout: *attemptPortmapTimeout,
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
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func peerVisitorCmd(ctx context.Context, args []string) {
	fs := flag.NewFlagSet("peer visitor", flag.ExitOnError)
	coordAddr := fs.String("coord", "127.0.0.1:7000", "coordinator address")
	controlProto := fs.String("control-proto", "tcp", "control plane protocol: tcp|kcp|quic")
	proxy := fs.String("proxy", "", "proxy name")
	secret := fs.String("secret", "", "secret key")
	user := fs.String("user", "visitor", "user name")
	dataProto := fs.String("data-proto", "quic", "data plane protocol: kcp|quic")
	quicCC := fs.String("quic-cc", "bbr", "quic congestion control: bbr|brutal (only applies when --data-proto=quic)")
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
	_ = fs.Parse(args)

	em := event.NewEmitter(os.Stdout, "peer-visitor")
	cfg := peer.VisitorConfig{
		CoordAddr:             *coordAddr,
		ControlProto:          control.Protocol(*controlProto),
		User:                  *user,
		ProxyName:             *proxy,
		SecretKey:             *secret,
		DataProto:             *dataProto,
		QuicCC:                *quicCC,
		Payload:               []byte(*payload),
		StunServers:           splitComma(*stunServers),
		StunTimeout:           *stunTimeout,
		GatherTimeout:         *gatherTimeout,
		AttemptV6Timeout:      *attemptV6Timeout,
		AttemptPortmapTimeout: *attemptPortmapTimeout,
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
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func stunCmd(ctx context.Context, args []string) {
	fs := flag.NewFlagSet("stun", flag.ExitOnError)
	var listens multiString
	fs.Var(&listens, "listen", "listen address (repeatable)")
	_ = fs.Parse(args)

	if len(listens) == 0 {
		listens = append(listens, "0.0.0.0:3478", "0.0.0.0:3479")
	}

	s, err := stun.ListenAndServe(ctx, listens)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	<-ctx.Done()
	s.Close()
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
