package punch

import (
	"context"
	"fmt"
	"net"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/udpowner"
	legacywire "github.com/miopunch/miopunch/internal/wire"
)

func testUDPSnapshot(addr string) UDPSnapshot {
	return UDPSnapshot{DirectAddrs: []string{addr}}
}

func testUDPDecision(sid string, localPeerAddr string, remotePeerAddr string) UDPDecision {
	return UDPDecision{
		LocalResponse: legacywire.NatHoleResp{
			TransactionID:   sid,
			Sid:             sid,
			Protocol:        "kcp",
			P2PNetwork:      "udp_only",
			PeerDirectAddrs: []string{localPeerAddr},
		},
		RemoteResponse: legacywire.NatHoleResp{
			TransactionID:   sid,
			Sid:             sid,
			Protocol:        "kcp",
			P2PNetwork:      "udp_only",
			PeerDirectAddrs: []string{remotePeerAddr},
		},
		AnalysisKey: "test-analysis-key",
		AnalyzerKey: "test-analyzer-key",
	}
}

func testGatherUDPSnapshot(ctx context.Context, cfg LoadedConfig, sid string) (UDPSnapshot, error) {
	if len(cfg.LocalCandidates) == 0 {
		return UDPSnapshot{}, fmt.Errorf("missing local candidates for %s", sid)
	}
	return normalizeUDPSnapshot(UDPSnapshot{DirectAddrs: []string{cfg.LocalCandidates[0].Addr}})
}

func testUDPAttempt(
	ctx context.Context,
	sid string,
	key []byte,
	udp4Conn *net.UDPConn,
	udp6Conn *net.UDPConn,
	resp *legacywire.NatHoleResp,
	udp4Demux *udpowner.TraversalDemux,
	udp6Demux *udpowner.TraversalDemux,
) (*connectivity.AttemptResult, error) {
	_ = udp6Conn
	_ = udp4Demux
	_ = udp6Demux
	if resp == nil || len(resp.PeerDirectAddrs) == 0 {
		return nil, fmt.Errorf("missing peer direct address for %s", sid)
	}
	remote, err := net.ResolveUDPAddr("udp", resp.PeerDirectAddrs[0])
	if err != nil {
		return nil, err
	}
	return &connectivity.AttemptResult{Path: PathDirectIPv4, Conn: udp4Conn, Remote: remote}, nil
}
