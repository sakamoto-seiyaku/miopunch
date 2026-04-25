package mqtt

import (
	"encoding/json"
	"slices"
	"testing"

	"github.com/miopunch/miopunch/internal/wire"
)

func TestJSONRoundTrip_NatHoleClientPreservesTCPInfo(t *testing.T) {
	in := &wire.NatHoleClient{
		TransactionID: "client-tx",
		ProxyName:     "proxy",
		Sid:           "sid",
		Capabilities: []string{
			wire.CapabilityTCPP2PV0,
		},
		P2PNetwork: "tcp_only",
		TCPDirectAddrs: []string{
			"192.0.2.10:41000",
		},
		TCPMappedAddrs: []string{
			"203.0.113.10:42000",
		},
		TCPSTUNCN: &wire.STUNViewObservation{
			Available:     true,
			OkCount:       2,
			RTTMs:         9,
			NATDifficulty: 1,
			MappedAddrs: []string{
				"203.0.113.10:42000",
			},
		},
		TCPSTUNGlobal: &wire.STUNViewObservation{
			Available:     false,
			NATDifficulty: 999,
			Errors: []string{
				"global unavailable",
			},
		},
	}

	var got wire.NatHoleClient
	roundTripJSON(t, "NatHoleClient", in, &got)

	assertStringSlice(t, "jsonRoundTrip(NatHoleClient).Capabilities", got.Capabilities, in.Capabilities)
	if got.P2PNetwork != in.P2PNetwork {
		t.Fatalf("jsonRoundTrip(NatHoleClient).P2PNetwork = %q, want %q", got.P2PNetwork, in.P2PNetwork)
	}
	assertStringSlice(t, "jsonRoundTrip(NatHoleClient).TCPDirectAddrs", got.TCPDirectAddrs, in.TCPDirectAddrs)
	assertStringSlice(t, "jsonRoundTrip(NatHoleClient).TCPMappedAddrs", got.TCPMappedAddrs, in.TCPMappedAddrs)
	assertSTUNViewObservation(t, "jsonRoundTrip(NatHoleClient).TCPSTUNCN", got.TCPSTUNCN, in.TCPSTUNCN)
	assertSTUNViewObservation(t, "jsonRoundTrip(NatHoleClient).TCPSTUNGlobal", got.TCPSTUNGlobal, in.TCPSTUNGlobal)
}

func TestJSONRoundTrip_NatHoleVisitorPreservesTCPInfo(t *testing.T) {
	in := &wire.NatHoleVisitor{
		TransactionID: "visitor-tx",
		ProxyName:     "proxy",
		Capabilities: []string{
			wire.CapabilityTCPP2PV0,
		},
		P2PNetwork: "auto",
		TCPDirectAddrs: []string{
			"192.0.2.20:51000",
		},
		TCPMappedAddrs: []string{
			"203.0.113.20:52000",
		},
		TCPSTUNCN: &wire.STUNViewObservation{
			Available:     false,
			NATDifficulty: 999,
			Errors: []string{
				"cn unavailable",
			},
		},
		TCPSTUNGlobal: &wire.STUNViewObservation{
			Available:     true,
			OkCount:       3,
			RTTMs:         12,
			NATDifficulty: 1,
			MappedAddrs: []string{
				"203.0.113.20:52000",
			},
		},
	}

	var got wire.NatHoleVisitor
	roundTripJSON(t, "NatHoleVisitor", in, &got)

	assertStringSlice(t, "jsonRoundTrip(NatHoleVisitor).Capabilities", got.Capabilities, in.Capabilities)
	if got.P2PNetwork != in.P2PNetwork {
		t.Fatalf("jsonRoundTrip(NatHoleVisitor).P2PNetwork = %q, want %q", got.P2PNetwork, in.P2PNetwork)
	}
	assertStringSlice(t, "jsonRoundTrip(NatHoleVisitor).TCPDirectAddrs", got.TCPDirectAddrs, in.TCPDirectAddrs)
	assertStringSlice(t, "jsonRoundTrip(NatHoleVisitor).TCPMappedAddrs", got.TCPMappedAddrs, in.TCPMappedAddrs)
	assertSTUNViewObservation(t, "jsonRoundTrip(NatHoleVisitor).TCPSTUNCN", got.TCPSTUNCN, in.TCPSTUNCN)
	assertSTUNViewObservation(t, "jsonRoundTrip(NatHoleVisitor).TCPSTUNGlobal", got.TCPSTUNGlobal, in.TCPSTUNGlobal)
}

func TestJSONRoundTrip_NatHoleRespPreservesTCPInfo(t *testing.T) {
	in := &wire.NatHoleResp{
		TransactionID: "resp-tx",
		Sid:           "sid",
		P2PNetwork:    "tcp_only",
		PeerTCPDirectAddrs: []string{
			"192.0.2.10:41000",
		},
		TCPCandidateAddrs: []string{
			"203.0.113.10:42100",
			"203.0.113.20:52100",
		},
		TCPSelectedView:    "global",
		TCPSelectedReason:  "availability",
		TCPPunchingEnabled: true,
		TCPPunchingError:   "mode converged",
		TCPDetectBehavior: &wire.TcpDetectBehavior{
			Role:          "receiver",
			Mode:          4,
			SendDelayMs:   25,
			ReadTimeoutMs: 1500,
			CandidatePorts: []wire.PortsRange{
				{From: 42100, To: 42104},
			},
			SendRandomPorts:   12,
			ListenRandomPorts: 6,
		},
	}

	var got wire.NatHoleResp
	roundTripJSON(t, "NatHoleResp", in, &got)

	if got.P2PNetwork != in.P2PNetwork {
		t.Fatalf("jsonRoundTrip(NatHoleResp).P2PNetwork = %q, want %q", got.P2PNetwork, in.P2PNetwork)
	}
	assertStringSlice(t, "jsonRoundTrip(NatHoleResp).PeerTCPDirectAddrs", got.PeerTCPDirectAddrs, in.PeerTCPDirectAddrs)
	assertStringSlice(t, "jsonRoundTrip(NatHoleResp).TCPCandidateAddrs", got.TCPCandidateAddrs, in.TCPCandidateAddrs)
	if got.TCPSelectedView != in.TCPSelectedView {
		t.Fatalf("jsonRoundTrip(NatHoleResp).TCPSelectedView = %q, want %q", got.TCPSelectedView, in.TCPSelectedView)
	}
	if got.TCPSelectedReason != in.TCPSelectedReason {
		t.Fatalf("jsonRoundTrip(NatHoleResp).TCPSelectedReason = %q, want %q", got.TCPSelectedReason, in.TCPSelectedReason)
	}
	if got.TCPPunchingEnabled != in.TCPPunchingEnabled {
		t.Fatalf("jsonRoundTrip(NatHoleResp).TCPPunchingEnabled = %v, want %v", got.TCPPunchingEnabled, in.TCPPunchingEnabled)
	}
	if got.TCPPunchingError != in.TCPPunchingError {
		t.Fatalf("jsonRoundTrip(NatHoleResp).TCPPunchingError = %q, want %q", got.TCPPunchingError, in.TCPPunchingError)
	}
	assertTCPDetectBehavior(t, "jsonRoundTrip(NatHoleResp).TCPDetectBehavior", got.TCPDetectBehavior, in.TCPDetectBehavior)
}

func roundTripJSON(t *testing.T, name string, in, out any) {
	t.Helper()

	data, err := json.Marshal(in)
	if err != nil {
		t.Fatalf("json.Marshal(%s) error = %v, want nil", name, err)
	}
	if err := json.Unmarshal(data, out); err != nil {
		t.Fatalf("json.Unmarshal(%s) error = %v, want nil", name, err)
	}
}

func assertStringSlice(t *testing.T, field string, got, want []string) {
	t.Helper()

	if !slices.Equal(got, want) {
		t.Fatalf("%s = %v, want %v", field, got, want)
	}
}

func assertSTUNViewObservation(t *testing.T, field string, got, want *wire.STUNViewObservation) {
	t.Helper()

	if got == nil || want == nil {
		if got != want {
			t.Fatalf("%s = %#v, want %#v", field, got, want)
		}
		return
	}
	if got.Available != want.Available {
		t.Fatalf("%s.Available = %v, want %v", field, got.Available, want.Available)
	}
	if got.OkCount != want.OkCount {
		t.Fatalf("%s.OkCount = %d, want %d", field, got.OkCount, want.OkCount)
	}
	if got.RTTMs != want.RTTMs {
		t.Fatalf("%s.RTTMs = %d, want %d", field, got.RTTMs, want.RTTMs)
	}
	if got.NATDifficulty != want.NATDifficulty {
		t.Fatalf("%s.NATDifficulty = %d, want %d", field, got.NATDifficulty, want.NATDifficulty)
	}
	assertStringSlice(t, field+".MappedAddrs", got.MappedAddrs, want.MappedAddrs)
	assertStringSlice(t, field+".Errors", got.Errors, want.Errors)
}

func assertTCPDetectBehavior(t *testing.T, field string, got, want *wire.TcpDetectBehavior) {
	t.Helper()

	if got == nil || want == nil {
		if got != want {
			t.Fatalf("%s = %#v, want %#v", field, got, want)
		}
		return
	}
	if got.Role != want.Role {
		t.Fatalf("%s.Role = %q, want %q", field, got.Role, want.Role)
	}
	if got.Mode != want.Mode {
		t.Fatalf("%s.Mode = %d, want %d", field, got.Mode, want.Mode)
	}
	if got.SendDelayMs != want.SendDelayMs {
		t.Fatalf("%s.SendDelayMs = %d, want %d", field, got.SendDelayMs, want.SendDelayMs)
	}
	if got.ReadTimeoutMs != want.ReadTimeoutMs {
		t.Fatalf("%s.ReadTimeoutMs = %d, want %d", field, got.ReadTimeoutMs, want.ReadTimeoutMs)
	}
	if !slices.Equal(got.CandidatePorts, want.CandidatePorts) {
		t.Fatalf("%s.CandidatePorts = %v, want %v", field, got.CandidatePorts, want.CandidatePorts)
	}
	if got.SendRandomPorts != want.SendRandomPorts {
		t.Fatalf("%s.SendRandomPorts = %d, want %d", field, got.SendRandomPorts, want.SendRandomPorts)
	}
	if got.ListenRandomPorts != want.ListenRandomPorts {
		t.Fatalf("%s.ListenRandomPorts = %d, want %d", field, got.ListenRandomPorts, want.ListenRandomPorts)
	}
}
