package connectivity

import (
	"context"
	"errors"
	"fmt"
	"net"
	"slices"
	"time"

	"github.com/pion/stun/v2"

	"github.com/miopunch/miopunch/xtcp/nathole"
)

const stunResponseTimeout = 3 * time.Second

type STUNDiscoveryResult struct {
	MappedAddrs []string
	Errors      []string
}

func DiscoverSTUN(ctx context.Context, conn *net.UDPConn, stunServers []string) STUNDiscoveryResult {
	res := STUNDiscoveryResult{
		MappedAddrs: make([]string, 0, len(stunServers)*2),
		Errors:      make([]string, 0),
	}
	for _, server := range stunServers {
		addrs, err := discoverFromSTUNServer(ctx, conn, server)
		if err != nil {
			res.Errors = append(res.Errors, fmt.Sprintf("%s: %v", server, err))
			continue
		}
		res.MappedAddrs = append(res.MappedAddrs, addrs...)
	}

	if len(res.MappedAddrs) > 4 {
		res.MappedAddrs = res.MappedAddrs[:4]
	}
	return res
}

func discoverFromSTUNServer(ctx context.Context, conn *net.UDPConn, addr string) ([]string, error) {
	external, other, err := doSTUNRequest(ctx, conn, addr)
	if err != nil {
		return nil, err
	}
	if external == "" {
		return nil, errors.New("no external address found")
	}

	out := make([]string, 0, 2)
	out = append(out, external)
	if other == "" {
		return out, nil
	}

	external2, _, err := doSTUNRequest(ctx, conn, other)
	if err != nil {
		return out, nil
	}
	if external2 != "" {
		out = append(out, external2)
	}
	return out, nil
}

func doSTUNRequest(ctx context.Context, conn *net.UDPConn, addr string) (externalAddr string, otherAddr string, err error) {
	raddr, err := net.ResolveUDPAddr("udp4", addr)
	if err != nil {
		return "", "", err
	}

	req, err := stun.Build(stun.TransactionID, stun.BindingRequest)
	if err != nil {
		return "", "", err
	}
	if err := req.NewTransactionID(); err != nil {
		return "", "", err
	}

	if _, err := conn.WriteToUDP(req.Raw, raddr); err != nil {
		return "", "", err
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
				return "", "", ctx.Err()
			}
			return "", "", readErr
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

	xor := &stun.XORMappedAddress{}
	mapped := &stun.MappedAddress{}
	changed := &nathole.ChangedAddress{}
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
	return externalAddr, otherAddr, nil
}
