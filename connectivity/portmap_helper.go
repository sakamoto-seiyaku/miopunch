package connectivity

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/huin/goupnp/dcps/internetgateway1"
	"github.com/huin/goupnp/dcps/internetgateway2"
	natpmp "github.com/jackpal/go-nat-pmp"
)

type PortMapAttemptResult struct {
	Method     string
	Candidates []netip.AddrPort
	Duration   time.Duration
	Err        error
}

type PortMapResult struct {
	Candidates []netip.AddrPort
	Attempts   []PortMapAttemptResult
}

type PortMapCleanup func(context.Context) error

type PortMapOptions struct {
	InternalPort int
	Lease        time.Duration
}

type portMapperFunc func(context.Context, int, time.Duration) (PortMapAttemptResult, PortMapCleanup)

func PortMapLease(sessionOverallTimeout time.Duration) time.Duration {
	lease := sessionOverallTimeout + 2*time.Minute
	if lease > 5*time.Minute {
		lease = 5 * time.Minute
	}
	if lease < 30*time.Second {
		lease = 30 * time.Second
	}
	return lease
}

func RunPortMap(ctx context.Context, opts PortMapOptions) (PortMapResult, PortMapCleanup) {
	return runPortMap(ctx, opts, []portMapperFunc{
		portmapUPnP,
		portmapNATPMP,
	})
}

func runPortMap(ctx context.Context, opts PortMapOptions, mappers []portMapperFunc) (PortMapResult, PortMapCleanup) {
	var (
		result  PortMapResult
		cleanup = func(context.Context) error { return nil }
	)
	if opts.InternalPort <= 0 {
		result.Attempts = append(result.Attempts, PortMapAttemptResult{
			Method: "portmap",
			Err:    fmt.Errorf("invalid internal port: %d", opts.InternalPort),
		})
		return result, cleanup
	}
	if opts.Lease <= 0 {
		opts.Lease = 5 * time.Minute
	}

	type attempt struct {
		res   PortMapAttemptResult
		unmap PortMapCleanup
	}
	attempts := make([]attempt, 0, len(mappers))
	for _, mapper := range mappers {
		res, unmap := mapper(ctx, opts.InternalPort, opts.Lease)
		attempts = append(attempts, attempt{res: res, unmap: unmap})
	}

	seen := make(map[netip.AddrPort]struct{})
	for _, a := range attempts {
		result.Attempts = append(result.Attempts, a.res)
		for _, ap := range a.res.Candidates {
			if _, ok := seen[ap]; ok {
				continue
			}
			seen[ap] = struct{}{}
			result.Candidates = append(result.Candidates, ap)
		}
	}
	result.Candidates = TrimDirectAddrPorts(result.Candidates)

	cleanup = func(cctx context.Context) error {
		var first error
		for _, a := range attempts {
			if a.unmap == nil {
				continue
			}
			if err := a.unmap(cctx); err != nil && first == nil {
				first = err
			}
		}
		return first
	}
	return result, cleanup
}

func portmapUPnP(ctx context.Context, internalPort int, lease time.Duration) (PortMapAttemptResult, PortMapCleanup) {
	return portmapUPnPWithProto(ctx, internalPort, lease, "UDP", "upnp")
}

func portmapUPnPTCP(ctx context.Context, internalPort int, lease time.Duration) (PortMapAttemptResult, PortMapCleanup) {
	return portmapUPnPWithProto(ctx, internalPort, lease, "TCP", "upnp_tcp")
}

func portmapUPnPWithProto(ctx context.Context, internalPort int, lease time.Duration, proto string, method string) (PortMapAttemptResult, PortMapCleanup) {
	start := time.Now()
	res := PortMapAttemptResult{Method: method}
	unmaps := make([]PortMapCleanup, 0)

	internalClientIP, err := guessOutboundIPv4()
	if err != nil {
		res.Err = fmt.Errorf("guess outbound ipv4: %w", err)
		res.Duration = time.Since(start)
		return res, nil
	}

	leaseSeconds := int(lease.Seconds())
	desc := "miopunch-p2p"

	addMapping := func(extIP string, externalPort uint16, unmapFn PortMapCleanup) {
		ap, err := netip.ParseAddrPort(net.JoinHostPort(extIP, strconv.Itoa(int(externalPort))))
		if err != nil {
			return
		}
		res.Candidates = append(res.Candidates, ap)
		if unmapFn != nil {
			unmaps = append(unmaps, unmapFn)
		}
	}

	upnpErr := func(err error) {
		if err == nil {
			return
		}
		if res.Err == nil {
			res.Err = err
		} else {
			res.Err = fmt.Errorf("%v; %w", res.Err, err)
		}
	}

	clients1, _, err := internetgateway1.NewWANIPConnection1Clients()
	if err != nil {
		upnpErr(err)
	} else {
		for _, c := range clients1 {
			extIP, err := c.GetExternalIPAddress()
			if err != nil {
				upnpErr(err)
				continue
			}
			extPort := uint16(internalPort)
			if err := c.AddPortMapping("", extPort, proto, uint16(internalPort), internalClientIP.String(), true, desc, uint32(leaseSeconds)); err != nil {
				upnpErr(err)
				continue
			}
			addMapping(extIP, extPort, func(_ context.Context) error {
				// Best-effort; no context support in goupnp.
				return c.DeletePortMapping("", extPort, proto)
			})
		}
	}

	ppp1, _, err := internetgateway1.NewWANPPPConnection1Clients()
	if err != nil {
		upnpErr(err)
	} else {
		for _, c := range ppp1 {
			extIP, err := c.GetExternalIPAddress()
			if err != nil {
				upnpErr(err)
				continue
			}
			extPort := uint16(internalPort)
			if err := c.AddPortMapping("", extPort, proto, uint16(internalPort), internalClientIP.String(), true, desc, uint32(leaseSeconds)); err != nil {
				upnpErr(err)
				continue
			}
			addMapping(extIP, extPort, func(_ context.Context) error {
				return c.DeletePortMapping("", extPort, proto)
			})
		}
	}

	clients2, _, err := internetgateway2.NewWANIPConnection1Clients()
	if err != nil {
		upnpErr(err)
	} else {
		for _, c := range clients2 {
			extIP, err := c.GetExternalIPAddress()
			if err != nil {
				upnpErr(err)
				continue
			}
			extPort := uint16(internalPort)
			if err := c.AddPortMapping("", extPort, proto, uint16(internalPort), internalClientIP.String(), true, desc, uint32(leaseSeconds)); err != nil {
				upnpErr(err)
				continue
			}
			addMapping(extIP, extPort, func(_ context.Context) error {
				return c.DeletePortMapping("", extPort, proto)
			})
		}
	}

	ppp2, _, err := internetgateway2.NewWANPPPConnection1Clients()
	if err != nil {
		upnpErr(err)
	} else {
		for _, c := range ppp2 {
			extIP, err := c.GetExternalIPAddress()
			if err != nil {
				upnpErr(err)
				continue
			}
			extPort := uint16(internalPort)
			if err := c.AddPortMapping("", extPort, proto, uint16(internalPort), internalClientIP.String(), true, desc, uint32(leaseSeconds)); err != nil {
				upnpErr(err)
				continue
			}
			addMapping(extIP, extPort, func(_ context.Context) error {
				return c.DeletePortMapping("", extPort, proto)
			})
		}
	}

	res.Candidates = TrimDirectAddrPorts(res.Candidates)
	res.Duration = time.Since(start)
	if len(res.Candidates) == 0 && res.Err == nil {
		res.Err = errors.New("no upnp candidates")
	}

	return res, func(cctx context.Context) error {
		var first error
		for _, fn := range unmaps {
			if fn == nil {
				continue
			}
			if err := fn(cctx); err != nil && first == nil {
				first = err
			}
		}
		return first
	}
}

func portmapNATPMP(ctx context.Context, internalPort int, lease time.Duration) (PortMapAttemptResult, PortMapCleanup) {
	return portmapNATPMPWithProto(ctx, internalPort, lease, "udp", "natpmp")
}

func portmapNATPMPTCP(ctx context.Context, internalPort int, lease time.Duration) (PortMapAttemptResult, PortMapCleanup) {
	return portmapNATPMPWithProto(ctx, internalPort, lease, "tcp", "natpmp_tcp")
}

func portmapNATPMPWithProto(ctx context.Context, internalPort int, lease time.Duration, proto string, method string) (PortMapAttemptResult, PortMapCleanup) {
	start := time.Now()
	res := PortMapAttemptResult{Method: method}

	gw, err := defaultGatewayIPv4()
	if err != nil {
		res.Err = err
		res.Duration = time.Since(start)
		return res, nil
	}

	client := natpmp.NewClient(gw)

	ext, err := client.GetExternalAddress()
	if err != nil {
		res.Err = err
		res.Duration = time.Since(start)
		return res, nil
	}
	extIP := net.IP(ext.ExternalIPAddress[:])
	extAddr, ok := netip.AddrFromSlice(extIP)
	if !ok || !extAddr.Is4() {
		res.Err = fmt.Errorf("unexpected external ip: %v", extIP)
		res.Duration = time.Since(start)
		return res, nil
	}

	lifetime := int(lease.Seconds())
	if lifetime < 30 {
		lifetime = 30
	}

	mapping, err := client.AddPortMapping(proto, internalPort, internalPort, lifetime)
	if err != nil {
		res.Err = err
		res.Duration = time.Since(start)
		return res, nil
	}

	ap := netip.AddrPortFrom(extAddr, uint16(mapping.MappedExternalPort))
	res.Candidates = TrimDirectAddrPorts([]netip.AddrPort{ap})
	res.Duration = time.Since(start)

	return res, func(_ context.Context) error {
		// NAT-PMP uses lifetime=0 to delete.
		_, err := client.AddPortMapping(proto, internalPort, internalPort, 0)
		return err
	}
}

func guessOutboundIPv4() (net.IP, error) {
	c, err := net.Dial("udp4", "8.8.8.8:80")
	if err != nil {
		return nil, err
	}
	defer c.Close()
	ua, ok := c.LocalAddr().(*net.UDPAddr)
	if !ok || ua.IP == nil || ua.IP.To4() == nil {
		return nil, errors.New("failed to determine outbound ipv4")
	}
	return ua.IP.To4(), nil
}

func defaultGatewayIPv4() (net.IP, error) {
	// Parse /proc/net/route to find the default route gateway.
	// Format: Iface Destination Gateway Flags RefCnt Use Metric Mask ...
	// Destination == 00000000 means default.
	b, err := os.ReadFile("/proc/net/route")
	if err != nil {
		return nil, err
	}
	lines := strings.Split(string(b), "\n")
	for _, line := range lines[1:] { // skip header
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		fields := strings.Fields(line)
		if len(fields) < 3 {
			continue
		}
		if fields[1] != "00000000" {
			continue
		}

		gwHex := fields[2]
		v, err := strconv.ParseUint(gwHex, 16, 32)
		if err != nil {
			continue
		}
		var ip [4]byte
		binary.LittleEndian.PutUint32(ip[:], uint32(v))
		return net.IPv4(ip[0], ip[1], ip[2], ip[3]).To4(), nil
	}
	return nil, errors.New("default gateway not found")
}
