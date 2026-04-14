package connectivity

import (
	"context"
	"errors"
	"fmt"
	"net"
	"slices"
	"time"

	"github.com/pion/stun/v2"

	"github.com/miopunch/miopunch/nat"
)

const stunResponseTimeout = 3 * time.Second

type STUNDiscoveryResult struct {
	MappedAddrs []string
	Errors      []string
	OkCount     int
	RTTMs       int
}

func DiscoverSTUN(ctx context.Context, conn *net.UDPConn, stunServers []string) STUNDiscoveryResult {
	res := STUNDiscoveryResult{
		MappedAddrs: make([]string, 0, len(stunServers)*2),
		Errors:      make([]string, 0),
	}
	var minRTT time.Duration
	for _, server := range stunServers {
		addrs, rtt, err := discoverFromSTUNServer(ctx, conn, server)
		if err != nil {
			res.Errors = append(res.Errors, fmt.Sprintf("%s: %v", server, err))
			continue
		}
		res.OkCount++
		if minRTT == 0 || (rtt > 0 && rtt < minRTT) {
			minRTT = rtt
		}
		res.MappedAddrs = append(res.MappedAddrs, addrs...)
	}
	if minRTT > 0 {
		res.RTTMs = int(minRTT.Milliseconds())
	}

	if len(res.MappedAddrs) > 4 {
		res.MappedAddrs = res.MappedAddrs[:4]
	}
	return res
}

func discoverFromSTUNServer(ctx context.Context, conn *net.UDPConn, addr string) ([]string, time.Duration, error) {
	external, other, rtt, err := doSTUNRequest(ctx, conn, addr)
	if err != nil {
		return nil, 0, err
	}
	if external == "" {
		return nil, 0, errors.New("no external address found")
	}

	out := make([]string, 0, 2)
	out = append(out, external)
	if other == "" {
		return out, rtt, nil
	}

	external2, _, _, err := doSTUNRequest(ctx, conn, other)
	if err != nil {
		return out, rtt, nil
	}
	if external2 != "" {
		out = append(out, external2)
	}
	return out, rtt, nil
}

func doSTUNRequest(ctx context.Context, conn *net.UDPConn, addr string) (externalAddr string, otherAddr string, rtt time.Duration, err error) {
	raddr, err := net.ResolveUDPAddr("udp4", addr)
	if err != nil {
		return "", "", 0, err
	}

	req, err := stun.Build(stun.TransactionID, stun.BindingRequest)
	if err != nil {
		return "", "", 0, err
	}
	if err := req.NewTransactionID(); err != nil {
		return "", "", 0, err
	}

	start := time.Now()
	if _, err := conn.WriteToUDP(req.Raw, raddr); err != nil {
		return "", "", 0, err
	}

	deadline := time.Now().Add(stunResponseTimeout)
	if dl, ok := ctx.Deadline(); ok && dl.Before(deadline) {
		deadline = dl
	}

	var resp stun.Message
	buf := make([]byte, 2048)
	for {
		_ = conn.SetReadDeadline(deadline)
		n, _, readErr := conn.ReadFromUDP(buf)
		_ = conn.SetReadDeadline(time.Time{})
		if readErr != nil {
			if ctx.Err() != nil {
				return "", "", 0, ctx.Err()
			}
			return "", "", 0, readErr
		}

		resp.Raw = append(resp.Raw[:0], buf[:n]...)
		if err := resp.Decode(); err != nil {
			continue
		}
		if resp.Type.Method != stun.MethodBinding {
			continue
		}
		if !slices.Equal(resp.TransactionID[:], req.TransactionID[:]) {
			continue
		}
		break
	}
	rtt = time.Since(start)

	xor := &stun.XORMappedAddress{}
	mapped := &stun.MappedAddress{}
	changed := &nat.ChangedAddress{}
	other := &stun.OtherAddress{}

	if err := mapped.GetFrom(&resp); err == nil {
		externalAddr = mapped.String()
	}
	if err := xor.GetFrom(&resp); err == nil {
		externalAddr = xor.String()
	}
	if err := changed.GetFrom(&resp); err == nil {
		otherAddr = changed.String()
	}
	if err := other.GetFrom(&resp); err == nil {
		otherAddr = other.String()
	}
	return externalAddr, otherAddr, rtt, nil
}
