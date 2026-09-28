package wire

import (
	"bytes"
	"slices"
	"testing"
)

func TestWriteReadMsg_RoundTrip(t *testing.T) {
	buf := bytes.NewBuffer(nil)
	in := &PeerHello{
		Role: "client",
		User: "u",
		Capabilities: []string{
			CapabilityTCPP2PV0,
		},
		P2PNetwork: "auto",
		ProxyName:  "p",
		SecretKey:  "s",
		AllowUsers: []string{"*"},
	}
	if err := WriteMsg(buf, in); err != nil {
		t.Fatalf("WriteMsg: %v", err)
	}

	out, err := ReadMsg(buf)
	if err != nil {
		t.Fatalf("ReadMsg: %v", err)
	}
	hello, ok := out.(*PeerHello)
	if !ok {
		t.Fatalf("unexpected type: %T", out)
	}
	if hello.Role != in.Role || hello.ProxyName != in.ProxyName {
		t.Fatalf("mismatch: %#v vs %#v", hello, in)
	}
	if hello.P2PNetwork != in.P2PNetwork {
		t.Fatalf("P2PNetwork = %q, want %q", hello.P2PNetwork, in.P2PNetwork)
	}
	if !slices.Equal(hello.Capabilities, in.Capabilities) {
		t.Fatalf("Capabilities = %v, want %v", hello.Capabilities, in.Capabilities)
	}
}

func TestWriteReadMsg_RoundTrip_TCPFields(t *testing.T) {
	buf := bytes.NewBuffer(nil)
	in := &NatHoleVisitor{
		TransactionID: "tx",
		ProxyName:     "p",
		Capabilities: []string{
			CapabilityTCPP2PV0,
		},
		P2PNetwork: "tcp_only",
		TCPDirectAddrs: []string{
			"192.0.2.1:1111",
		},
		TCPAssistedAddrs: []string{
			"10.0.0.1:1111",
		},
		TCPMappedAddrs: []string{
			"203.0.113.1:40000",
		},
		TCPSTUNCN: &STUNViewObservation{
			Available:     true,
			OkCount:       2,
			RTTMs:         10,
			NATDifficulty: 1,
			MappedAddrs: []string{
				"203.0.113.1:40000",
			},
		},
		TCPSTUNGlobal: &STUNViewObservation{
			Available:     false,
			NATDifficulty: 999,
		},
	}
	if err := WriteMsg(buf, in); err != nil {
		t.Fatalf("WriteMsg: %v", err)
	}

	out, err := ReadMsg(buf)
	if err != nil {
		t.Fatalf("ReadMsg: %v", err)
	}
	got, ok := out.(*NatHoleVisitor)
	if !ok {
		t.Fatalf("unexpected type: %T", out)
	}

	if got.P2PNetwork != in.P2PNetwork {
		t.Fatalf("P2PNetwork = %q, want %q", got.P2PNetwork, in.P2PNetwork)
	}
	if !slices.Equal(got.Capabilities, in.Capabilities) {
		t.Fatalf("Capabilities = %v, want %v", got.Capabilities, in.Capabilities)
	}

	if !slices.Equal(got.TCPDirectAddrs, in.TCPDirectAddrs) {
		t.Fatalf("TCPDirectAddrs = %v, want %v", got.TCPDirectAddrs, in.TCPDirectAddrs)
	}
	if !slices.Equal(got.TCPAssistedAddrs, in.TCPAssistedAddrs) {
		t.Fatalf("TCPAssistedAddrs = %v, want %v", got.TCPAssistedAddrs, in.TCPAssistedAddrs)
	}
	if !slices.Equal(got.TCPMappedAddrs, in.TCPMappedAddrs) {
		t.Fatalf("TCPMappedAddrs = %v, want %v", got.TCPMappedAddrs, in.TCPMappedAddrs)
	}

	if got.TCPSTUNCN == nil {
		t.Fatalf("TCPSTUNCN = nil, want non-nil")
	}
	if got.TCPSTUNCN.Available != in.TCPSTUNCN.Available {
		t.Fatalf("TCPSTUNCN.Available = %v, want %v", got.TCPSTUNCN.Available, in.TCPSTUNCN.Available)
	}
	if !slices.Equal(got.TCPSTUNCN.MappedAddrs, in.TCPSTUNCN.MappedAddrs) {
		t.Fatalf("TCPSTUNCN.MappedAddrs = %v, want %v", got.TCPSTUNCN.MappedAddrs, in.TCPSTUNCN.MappedAddrs)
	}

	if got.TCPSTUNGlobal == nil {
		t.Fatalf("TCPSTUNGlobal = nil, want non-nil")
	}
	if got.TCPSTUNGlobal.Available != in.TCPSTUNGlobal.Available {
		t.Fatalf("TCPSTUNGlobal.Available = %v, want %v", got.TCPSTUNGlobal.Available, in.TCPSTUNGlobal.Available)
	}
	if got.TCPSTUNGlobal.NATDifficulty != in.TCPSTUNGlobal.NATDifficulty {
		t.Fatalf("TCPSTUNGlobal.NATDifficulty = %v, want %v", got.TCPSTUNGlobal.NATDifficulty, in.TCPSTUNGlobal.NATDifficulty)
	}
}

func TestWriteReadMsg_RoundTrip_TCPClientFields(t *testing.T) {
	buf := bytes.NewBuffer(nil)
	in := &NatHoleClient{
		TransactionID: "tx",
		ProxyName:     "p",
		Sid:           "sid",
		Capabilities: []string{
			CapabilityTCPP2PV0,
		},
		P2PNetwork: "tcp_only",
		TCPDirectAddrs: []string{
			"192.0.2.2:2222",
		},
		TCPAssistedAddrs: []string{
			"10.0.0.2:2222",
		},
		TCPMappedAddrs: []string{
			"203.0.113.2:50000",
		},
		TCPSTUNCN: &STUNViewObservation{
			Available:     true,
			OkCount:       3,
			RTTMs:         11,
			NATDifficulty: 2,
			MappedAddrs: []string{
				"203.0.113.2:50000",
			},
			Errors: []string{
				"stun cn warning",
			},
		},
		TCPSTUNGlobal: &STUNViewObservation{
			Available:     true,
			OkCount:       4,
			RTTMs:         12,
			NATDifficulty: 1,
			MappedAddrs: []string{
				"198.51.100.2:51000",
			},
		},
	}
	if err := WriteMsg(buf, in); err != nil {
		t.Fatalf("WriteMsg(NatHoleClient) error = %v, want nil", err)
	}

	out, err := ReadMsg(buf)
	if err != nil {
		t.Fatalf("ReadMsg(NatHoleClient) error = %v, want nil", err)
	}
	got, ok := out.(*NatHoleClient)
	if !ok {
		t.Fatalf("ReadMsg(NatHoleClient) type = %T, want *NatHoleClient", out)
	}

	if got.P2PNetwork != in.P2PNetwork {
		t.Fatalf("ReadMsg(NatHoleClient).P2PNetwork = %q, want %q", got.P2PNetwork, in.P2PNetwork)
	}
	if !slices.Equal(got.Capabilities, in.Capabilities) {
		t.Fatalf("ReadMsg(NatHoleClient).Capabilities = %v, want %v", got.Capabilities, in.Capabilities)
	}
	if !slices.Equal(got.TCPDirectAddrs, in.TCPDirectAddrs) {
		t.Fatalf("ReadMsg(NatHoleClient).TCPDirectAddrs = %v, want %v", got.TCPDirectAddrs, in.TCPDirectAddrs)
	}
	if !slices.Equal(got.TCPAssistedAddrs, in.TCPAssistedAddrs) {
		t.Fatalf("ReadMsg(NatHoleClient).TCPAssistedAddrs = %v, want %v", got.TCPAssistedAddrs, in.TCPAssistedAddrs)
	}
	if !slices.Equal(got.TCPMappedAddrs, in.TCPMappedAddrs) {
		t.Fatalf("ReadMsg(NatHoleClient).TCPMappedAddrs = %v, want %v", got.TCPMappedAddrs, in.TCPMappedAddrs)
	}
	assertSTUNViewObservation(t, "ReadMsg(NatHoleClient).TCPSTUNCN", got.TCPSTUNCN, in.TCPSTUNCN)
	assertSTUNViewObservation(t, "ReadMsg(NatHoleClient).TCPSTUNGlobal", got.TCPSTUNGlobal, in.TCPSTUNGlobal)
}

func TestWriteReadMsg_RoundTrip_TCPRespFields(t *testing.T) {
	buf := bytes.NewBuffer(nil)
	in := &NatHoleResp{
		TransactionID: "tx",
		Sid:           "sid",
		P2PNetwork:    "tcp_only",
		PeerTCPDirectAddrs: []string{
			"192.0.2.1:1111",
			"192.0.2.2:2222",
		},
		TCPCandidateAddrs: []string{
			"203.0.113.1:40100",
			"203.0.113.2:50100",
		},
		TCPAssistedAddrs: []string{
			"10.0.0.1:1111",
			"10.0.0.2:2222",
		},
		TCPSelectedView:    "global",
		TCPSelectedReason:  "availability",
		TCPPunchingEnabled: true,
		TCPPunchingError:   "insufficient evidence",
		TCPDetectBehavior: &TcpDetectBehavior{
			Role:          "sender",
			Mode:          2,
			SendDelayMs:   15,
			ReadTimeoutMs: 1200,
			CandidatePorts: []PortsRange{
				{From: 40100, To: 40102},
				{From: 50100, To: 50102},
			},
			SendRandomPorts:   8,
			ListenRandomPorts: 4,
		},
	}
	if err := WriteMsg(buf, in); err != nil {
		t.Fatalf("WriteMsg(NatHoleResp) error = %v, want nil", err)
	}

	out, err := ReadMsg(buf)
	if err != nil {
		t.Fatalf("ReadMsg(NatHoleResp) error = %v, want nil", err)
	}
	got, ok := out.(*NatHoleResp)
	if !ok {
		t.Fatalf("ReadMsg(NatHoleResp) type = %T, want *NatHoleResp", out)
	}

	if got.P2PNetwork != in.P2PNetwork {
		t.Fatalf("ReadMsg(NatHoleResp).P2PNetwork = %q, want %q", got.P2PNetwork, in.P2PNetwork)
	}
	if !slices.Equal(got.PeerTCPDirectAddrs, in.PeerTCPDirectAddrs) {
		t.Fatalf("ReadMsg(NatHoleResp).PeerTCPDirectAddrs = %v, want %v", got.PeerTCPDirectAddrs, in.PeerTCPDirectAddrs)
	}
	if !slices.Equal(got.TCPCandidateAddrs, in.TCPCandidateAddrs) {
		t.Fatalf("ReadMsg(NatHoleResp).TCPCandidateAddrs = %v, want %v", got.TCPCandidateAddrs, in.TCPCandidateAddrs)
	}
	if !slices.Equal(got.TCPAssistedAddrs, in.TCPAssistedAddrs) {
		t.Fatalf("ReadMsg(NatHoleResp).TCPAssistedAddrs = %v, want %v", got.TCPAssistedAddrs, in.TCPAssistedAddrs)
	}
	if got.TCPSelectedView != in.TCPSelectedView {
		t.Fatalf("ReadMsg(NatHoleResp).TCPSelectedView = %q, want %q", got.TCPSelectedView, in.TCPSelectedView)
	}
	if got.TCPSelectedReason != in.TCPSelectedReason {
		t.Fatalf("ReadMsg(NatHoleResp).TCPSelectedReason = %q, want %q", got.TCPSelectedReason, in.TCPSelectedReason)
	}
	if got.TCPPunchingEnabled != in.TCPPunchingEnabled {
		t.Fatalf("ReadMsg(NatHoleResp).TCPPunchingEnabled = %v, want %v", got.TCPPunchingEnabled, in.TCPPunchingEnabled)
	}
	if got.TCPPunchingError != in.TCPPunchingError {
		t.Fatalf("ReadMsg(NatHoleResp).TCPPunchingError = %q, want %q", got.TCPPunchingError, in.TCPPunchingError)
	}
	assertTCPDetectBehavior(t, "ReadMsg(NatHoleResp).TCPDetectBehavior", got.TCPDetectBehavior, in.TCPDetectBehavior)
}

func assertSTUNViewObservation(t *testing.T, field string, got, want *STUNViewObservation) {
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
	if !slices.Equal(got.MappedAddrs, want.MappedAddrs) {
		t.Fatalf("%s.MappedAddrs = %v, want %v", field, got.MappedAddrs, want.MappedAddrs)
	}
	if !slices.Equal(got.Errors, want.Errors) {
		t.Fatalf("%s.Errors = %v, want %v", field, got.Errors, want.Errors)
	}
}

func assertTCPDetectBehavior(t *testing.T, field string, got, want *TcpDetectBehavior) {
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
