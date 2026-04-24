package connectivity

import (
	"context"
	"net"

	"github.com/miopunch/miopunch/internal/stunclient"
)

const (
	internalSTUNMaxConcurrency        = 3
	internalSTUNResolvedEndpointLimit = internalSTUNMaxConcurrency * 2
	internalSTUNMinMappedAddrs        = 2
	internalSTUNPreferredAddrs        = 3
)

type STUNDiscoveryResult struct {
	MappedAddrs []string
	Errors      []string
	OkCount     int
	RTTMs       int
}

func DiscoverSTUN(ctx context.Context, conn *net.UDPConn, stunServers []string) STUNDiscoveryResult {
	res := stunclient.DiscoverUDP(ctx, conn, stunServers)
	return STUNDiscoveryResult{
		MappedAddrs: res.MappedAddrs,
		Errors:      res.Errors,
		OkCount:     res.OkCount,
		RTTMs:       res.RTTMs,
	}
}

func discoverInternalSTUN(ctx context.Context, client *stunclient.UDPClient, stunServers []string) STUNDiscoveryResult {
	res := stunclient.DiscoverUDPWithClient(ctx, client, stunServers, internalSTUNMaxConcurrency, stunclient.StopFunc(shouldStopInternalSTUNSampling))
	return STUNDiscoveryResult{
		MappedAddrs: res.MappedAddrs,
		Errors:      res.Errors,
		OkCount:     res.OkCount,
		RTTMs:       res.RTTMs,
	}
}

func shouldStopInternalSTUNSampling(res stunclient.DiscoveryResult) bool {
	valid, _ := stunclient.SanitizeMappedAddrs(res.MappedAddrs)
	if len(valid) >= internalSTUNMinMappedAddrs && res.OkCount >= internalSTUNMinMappedAddrs {
		return true
	}

	uniq := make(map[string]struct{}, len(valid))
	for _, addr := range valid {
		uniq[addr] = struct{}{}
	}
	return len(uniq) >= internalSTUNPreferredAddrs
}
