package connectivity

import (
	"net"
	"strconv"
	"testing"

	"github.com/miopunch/miopunch/nat"
)

func TestDeriveUDPLocalSourceCandidatesSkipsLoopbackSource(t *testing.T) {
	runtimeConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4zero, Port: 0})
	if err != nil {
		t.Fatalf("ListenUDP(runtime) error = %v, want nil", err)
	}
	t.Cleanup(func() { runtimeConn.Close() })

	got := DeriveUDPLocalSourceCandidates(
		[]string{"127.0.0.1:9"},
		runtimeConn,
		nil,
		P2PIPFamilyV4,
	)
	if len(got) != 0 {
		t.Fatalf("DeriveUDPLocalSourceCandidates(loopback) = %v, want empty", got)
	}
}

func TestDeriveUDPLocalSourceCandidatesUsesRuntimePort(t *testing.T) {
	localIPs, err := nat.ListLocalIPsForNatHole(1)
	if err != nil {
		t.Fatalf("ListLocalIPsForNatHole() error = %v, want nil", err)
	}
	if len(localIPs) == 0 {
		t.Skip("no non-loopback local IPv4 address available")
	}

	serverIP := net.ParseIP(localIPs[0])
	if serverIP == nil {
		t.Fatalf("ParseIP(%q) = nil, want IP", localIPs[0])
	}
	server, err := net.ListenUDP("udp4", &net.UDPAddr{IP: serverIP, Port: 0})
	if err != nil {
		t.Fatalf("ListenUDP(server %s) error = %v, want nil", serverIP, err)
	}
	t.Cleanup(func() { server.Close() })

	runtimeConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4zero, Port: 0})
	if err != nil {
		t.Fatalf("ListenUDP(runtime) error = %v, want nil", err)
	}
	t.Cleanup(func() { runtimeConn.Close() })

	runtimeAddr, ok := runtimeConn.LocalAddr().(*net.UDPAddr)
	if !ok || runtimeAddr == nil {
		t.Fatalf("runtime LocalAddr() = %#v, want UDPAddr", runtimeConn.LocalAddr())
	}
	serverAddr, ok := server.LocalAddr().(*net.UDPAddr)
	if !ok || serverAddr == nil {
		t.Fatalf("server LocalAddr() = %#v, want UDPAddr", server.LocalAddr())
	}

	got := DeriveUDPLocalSourceCandidates(
		[]string{net.JoinHostPort(serverAddr.IP.String(), strconv.Itoa(serverAddr.Port))},
		runtimeConn,
		nil,
		P2PIPFamilyV4,
	)
	if len(got) != 1 {
		t.Fatalf("DeriveUDPLocalSourceCandidates(non-loopback) len = %d, want 1; got %v", len(got), got)
	}
	if got[0].Port() != uint16(runtimeAddr.Port) {
		t.Fatalf("DeriveUDPLocalSourceCandidates(non-loopback)[0].Port = %d, want runtime port %d", got[0].Port(), runtimeAddr.Port)
	}
}
