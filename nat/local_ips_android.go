// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//go:build android

package nat

import (
	"encoding/binary"
	"fmt"
	"net"
	"syscall"
	"unsafe"
)

func listAllLocalIPs() ([]net.IP, error) {
	tab, err := netlinkAddrRIBNoBind(syscall.AF_UNSPEC)
	if err != nil {
		return nil, err
	}
	msgs, err := syscall.ParseNetlinkMessage(tab)
	if err != nil {
		return nil, fmt.Errorf("parse netlink message: %w", err)
	}

	seen := map[string]struct{}{}
	ips := make([]net.IP, 0, len(msgs))
	for _, msg := range msgs {
		switch msg.Header.Type {
		case syscall.NLMSG_DONE:
			return ips, nil
		case syscall.RTM_NEWADDR:
			ip, ok, err := ipFromNetlinkAddrMessage(msg)
			if err != nil {
				return nil, err
			}
			if !ok {
				continue
			}
			key := ip.String()
			if _, exists := seen[key]; exists {
				continue
			}
			seen[key] = struct{}{}
			ips = append(ips, ip)
		}
	}
	return ips, nil
}

func netlinkAddrRIBNoBind(family int) ([]byte, error) {
	s, err := syscall.Socket(syscall.AF_NETLINK, syscall.SOCK_RAW|syscall.SOCK_CLOEXEC, syscall.NETLINK_ROUTE)
	if err != nil {
		return nil, fmt.Errorf("open netlink route socket: %w", err)
	}
	defer syscall.Close(s) // best-effort close

	req := newNetlinkAddrRequest(family)
	sa := &syscall.SockaddrNetlink{Family: syscall.AF_NETLINK}
	if err := syscall.Sendto(s, req, 0, sa); err != nil {
		return nil, fmt.Errorf("send rtm_getaddr: %w", err)
	}

	var out []byte
	buf := make([]byte, 32*1024)
	for {
		n, _, err := syscall.Recvfrom(s, buf, 0)
		if err != nil {
			return nil, fmt.Errorf("recv rtm_getaddr: %w", err)
		}
		if n < syscall.NLMSG_HDRLEN {
			return nil, fmt.Errorf("short netlink response: %d", n)
		}
		chunk := append([]byte(nil), buf[:n]...)
		out = append(out, chunk...)

		msgs, err := syscall.ParseNetlinkMessage(chunk)
		if err != nil {
			return nil, fmt.Errorf("parse netlink response chunk: %w", err)
		}
		for _, msg := range msgs {
			if msg.Header.Type == syscall.NLMSG_DONE {
				return out, nil
			}
			if msg.Header.Type == syscall.NLMSG_ERROR {
				return nil, fmt.Errorf("netlink returned NLMSG_ERROR")
			}
		}
	}
}

func newNetlinkAddrRequest(family int) []byte {
	reqLen := syscall.NLMSG_HDRLEN + syscall.SizeofRtGenmsg
	req := make([]byte, reqLen)
	binary.LittleEndian.PutUint32(req[0:4], uint32(reqLen))
	binary.LittleEndian.PutUint16(req[4:6], uint16(syscall.RTM_GETADDR))
	binary.LittleEndian.PutUint16(req[6:8], uint16(syscall.NLM_F_DUMP|syscall.NLM_F_REQUEST))
	binary.LittleEndian.PutUint32(req[8:12], 1)
	req[syscall.NLMSG_HDRLEN] = uint8(family)
	return req
}

func ipFromNetlinkAddrMessage(msg syscall.NetlinkMessage) (net.IP, bool, error) {
	if len(msg.Data) < syscall.SizeofIfAddrmsg {
		return nil, false, fmt.Errorf("short ifaddr message: %d", len(msg.Data))
	}
	ifam := (*syscall.IfAddrmsg)(unsafe.Pointer(&msg.Data[0]))
	attrs, err := syscall.ParseNetlinkRouteAttr(&msg)
	if err != nil {
		return nil, false, fmt.Errorf("parse netlink route attr: %w", err)
	}

	var fallback net.IP
	for _, attr := range attrs {
		switch attr.Attr.Type {
		case syscall.IFA_LOCAL:
			if ip, ok := ipFromRouteAttr(ifam.Family, attr.Value); ok {
				return ip, true, nil
			}
		case syscall.IFA_ADDRESS:
			if ip, ok := ipFromRouteAttr(ifam.Family, attr.Value); ok {
				fallback = ip
			}
		}
	}
	if fallback != nil {
		return fallback, true, nil
	}
	return nil, false, nil
}

func ipFromRouteAttr(family uint8, value []byte) (net.IP, bool) {
	switch family {
	case syscall.AF_INET:
		if len(value) < net.IPv4len {
			return nil, false
		}
		return net.IPv4(value[0], value[1], value[2], value[3]), true
	case syscall.AF_INET6:
		if len(value) < net.IPv6len {
			return nil, false
		}
		ip := make(net.IP, net.IPv6len)
		copy(ip, value[:net.IPv6len])
		return ip, true
	default:
		return nil, false
	}
}
