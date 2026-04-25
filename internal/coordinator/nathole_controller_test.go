package coordinator

import (
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/wire"
)

func TestControllerAnalysis_AllowsPunchingFallbackWithAssistedAddrs(t *testing.T) {
	c, err := NewController(time.Minute)
	if err != nil {
		t.Fatalf("NewController() error = %v, want nil", err)
	}

	session := &Session{
		sid: "sid-fallback-assisted",
		clientMsg: &wire.NatHoleClient{
			TransactionID: "c-tx",
			DirectAddrs:   []string{"[::1]:1"},
			MappedAddrs:   []string{"203.0.113.1:40000"},
			AssistedAddrs: []string{"192.168.0.10:11111"},
		},
		visitorMsg: &wire.NatHoleVisitor{
			TransactionID: "v-tx",
			DirectAddrs:   []string{"[::1]:2"},
			MappedAddrs:   []string{"203.0.113.2:50000"},
			AssistedAddrs: []string{"192.168.0.20:22222"},
		},
	}

	vResp, cResp, err := c.analysis(session)
	if err != nil {
		t.Fatalf("analysis() error = %v, want nil", err)
	}
	if vResp == nil || cResp == nil {
		t.Fatalf("analysis() returned nil resp: visitor=%#v client=%#v", vResp, cResp)
	}
	if !vResp.PunchingEnabled || !cResp.PunchingEnabled {
		t.Fatalf("analysis() PunchingEnabled = (visitor=%v client=%v), want both true", vResp.PunchingEnabled, cResp.PunchingEnabled)
	}
	if len(vResp.AssistedAddrs) != 1 || vResp.AssistedAddrs[0] != "192.168.0.10:11111" {
		t.Fatalf("analysis() visitor AssistedAddrs = %#v, want client assisted addrs preserved", vResp.AssistedAddrs)
	}
	if len(cResp.AssistedAddrs) != 1 || cResp.AssistedAddrs[0] != "192.168.0.20:22222" {
		t.Fatalf("analysis() client AssistedAddrs = %#v, want visitor assisted addrs preserved", cResp.AssistedAddrs)
	}
	if vResp.DetectBehavior.ReadTimeoutMs != 5000 || cResp.DetectBehavior.ReadTimeoutMs != 5000 {
		t.Fatalf("analysis() ReadTimeoutMs = (visitor=%d client=%d), want both 5000", vResp.DetectBehavior.ReadTimeoutMs, cResp.DetectBehavior.ReadTimeoutMs)
	}
}

func TestControllerAnalysis_SelectedViewDoesNotAffectAssistedAddrs(t *testing.T) {
	c, err := NewController(time.Minute)
	if err != nil {
		t.Fatalf("NewController() error = %v, want nil", err)
	}

	globalMappedClient := []string{"203.0.113.1:40000", "203.0.113.1:40001"}
	globalMappedVisitor := []string{"203.0.113.2:50000", "203.0.113.2:50001"}

	session := &Session{
		sid: "sid-selected-view-boundary",
		clientMsg: &wire.NatHoleClient{
			TransactionID: "c-tx",
			DirectAddrs:   []string{"[::1]:1"},
			MappedAddrs:   []string{"203.0.113.1:40000"},
			AssistedAddrs: []string{"192.168.0.10:11111"},
			STUNCN:        &wire.STUNViewObservation{Available: false, NATDifficulty: 999},
			STUNGlobal: &wire.STUNViewObservation{
				Available:     true,
				NATDifficulty: 1,
				RTTMs:         10,
				OkCount:       2,
				MappedAddrs:   globalMappedClient,
			},
		},
		visitorMsg: &wire.NatHoleVisitor{
			TransactionID: "v-tx",
			DirectAddrs:   []string{"[::1]:2"},
			MappedAddrs:   []string{"203.0.113.2:50000"},
			AssistedAddrs: []string{"192.168.0.20:22222"},
			STUNCN:        &wire.STUNViewObservation{Available: false, NATDifficulty: 999},
			STUNGlobal: &wire.STUNViewObservation{
				Available:     true,
				NATDifficulty: 1,
				RTTMs:         10,
				OkCount:       2,
				MappedAddrs:   globalMappedVisitor,
			},
		},
	}

	vResp, cResp, err := c.analysis(session)
	if err != nil {
		t.Fatalf("analysis() error = %v, want nil", err)
	}
	if vResp.SelectedView != "global" || vResp.SelectedReason != "availability" {
		t.Fatalf("analysis() visitor selected_view=(%q,%q), want (global,availability)", vResp.SelectedView, vResp.SelectedReason)
	}
	if cResp.SelectedView != "global" || cResp.SelectedReason != "availability" {
		t.Fatalf("analysis() client selected_view=(%q,%q), want (global,availability)", cResp.SelectedView, cResp.SelectedReason)
	}

	if len(vResp.AssistedAddrs) != 1 || vResp.AssistedAddrs[0] != "192.168.0.10:11111" {
		t.Fatalf("analysis() visitor AssistedAddrs = %#v, want client assisted addrs preserved", vResp.AssistedAddrs)
	}
	if len(cResp.AssistedAddrs) != 1 || cResp.AssistedAddrs[0] != "192.168.0.20:22222" {
		t.Fatalf("analysis() client AssistedAddrs = %#v, want visitor assisted addrs preserved", cResp.AssistedAddrs)
	}
	if len(vResp.CandidateAddrs) == 0 || len(cResp.CandidateAddrs) == 0 {
		t.Fatalf("analysis() CandidateAddrs = (visitor=%#v client=%#v), want non-empty", vResp.CandidateAddrs, cResp.CandidateAddrs)
	}
}

func TestControllerAnalysis_CompactCandidateCloneDoesNotBreakNATClassify(t *testing.T) {
	c, err := NewController(time.Minute)
	if err != nil {
		t.Fatalf("NewController() error = %v, want nil", err)
	}

	repeatedClientMapped := []string{
		"203.0.113.1:40000",
		"203.0.113.1:40000",
		"203.0.113.1:40000",
		"203.0.113.1:40000",
	}
	repeatedVisitorMapped := []string{
		"203.0.113.2:50000",
		"203.0.113.2:50000",
		"203.0.113.2:50000",
		"203.0.113.2:50000",
	}

	session := &Session{
		sid: "sid-compact-clone",
		clientMsg: &wire.NatHoleClient{
			TransactionID: "c-tx",
			DirectAddrs:   []string{"[::1]:1"},
			AssistedAddrs: []string{"192.168.0.10:11111"},
			STUNCN:        &wire.STUNViewObservation{Available: false, NATDifficulty: 999},
			STUNGlobal: &wire.STUNViewObservation{
				Available:     true,
				NATDifficulty: 1,
				RTTMs:         10,
				OkCount:       4,
				MappedAddrs:   repeatedClientMapped,
			},
		},
		visitorMsg: &wire.NatHoleVisitor{
			TransactionID: "v-tx",
			DirectAddrs:   []string{"[::1]:2"},
			AssistedAddrs: []string{"192.168.0.20:22222"},
			STUNCN:        &wire.STUNViewObservation{Available: false, NATDifficulty: 999},
			STUNGlobal: &wire.STUNViewObservation{
				Available:     true,
				NATDifficulty: 1,
				RTTMs:         10,
				OkCount:       4,
				MappedAddrs:   repeatedVisitorMapped,
			},
		},
	}

	vResp, cResp, err := c.analysis(session)
	if err != nil {
		t.Fatalf("analysis() error = %v, want nil", err)
	}
	if vResp == nil || cResp == nil {
		t.Fatalf("analysis() returned nil resp: visitor=%#v client=%#v", vResp, cResp)
	}
	if session.cNatFeature == nil || session.vNatFeature == nil {
		t.Fatalf("analysis() nat features = (visitor=%#v client=%#v), want both classified", session.vNatFeature, session.cNatFeature)
	}
	if len(vResp.CandidateAddrs) != 1 || len(cResp.CandidateAddrs) != 1 {
		t.Fatalf("analysis() compacted candidate_addrs = (visitor=%#v client=%#v), want one deduped addr each", vResp.CandidateAddrs, cResp.CandidateAddrs)
	}
}

func TestControllerAnalysis_EchoesAndDerivesTCPFields(t *testing.T) {
	c, err := NewController(time.Minute)
	if err != nil {
		t.Fatalf("NewController() error = %v, want nil", err)
	}

	session := &Session{
		sid: "sid-tcp-fields",
		clientMsg: &wire.NatHoleClient{
			TransactionID: "c-tx",
			DirectAddrs:   []string{"[::1]:1"},
			MappedAddrs:   []string{"203.0.113.1:40000", "203.0.113.1:40001"},
			AssistedAddrs: []string{"192.168.0.10:11111"},
			TCPDirectAddrs: []string{
				"192.0.2.1:1111",
				"192.0.2.1:1111",
				"invalid",
			},
			TCPMappedAddrs: []string{
				"203.0.113.10:41000",
				"203.0.113.10:41000",
			},
		},
		visitorMsg: &wire.NatHoleVisitor{
			TransactionID: "v-tx",
			DirectAddrs:   []string{"[::1]:2"},
			MappedAddrs:   []string{"203.0.113.2:50000", "203.0.113.2:50001"},
			AssistedAddrs: []string{"192.168.0.20:22222"},
			TCPDirectAddrs: []string{
				"192.0.2.2:2222",
			},
			TCPMappedAddrs: []string{
				"203.0.113.20:51000",
			},
		},
	}

	vResp, cResp, err := c.analysis(session)
	if err != nil {
		t.Fatalf("analysis() error = %v, want nil", err)
	}
	if vResp == nil || cResp == nil {
		t.Fatalf("analysis() returned nil resp: visitor=%#v client=%#v", vResp, cResp)
	}

	if !slices.Equal(vResp.PeerTCPDirectAddrs, []string{"192.0.2.1:1111"}) {
		t.Fatalf("analysis() visitor PeerTCPDirectAddrs = %#v, want %#v", vResp.PeerTCPDirectAddrs, []string{"192.0.2.1:1111"})
	}
	if !slices.Equal(cResp.PeerTCPDirectAddrs, []string{"192.0.2.2:2222"}) {
		t.Fatalf("analysis() client PeerTCPDirectAddrs = %#v, want %#v", cResp.PeerTCPDirectAddrs, []string{"192.0.2.2:2222"})
	}

	if !slices.Equal(vResp.TCPCandidateAddrs, []string{"203.0.113.10:41100"}) {
		t.Fatalf("analysis() visitor TCPCandidateAddrs = %#v, want %#v", vResp.TCPCandidateAddrs, []string{"203.0.113.10:41100"})
	}
	if !slices.Equal(cResp.TCPCandidateAddrs, []string{"203.0.113.20:51100"}) {
		t.Fatalf("analysis() client TCPCandidateAddrs = %#v, want %#v", cResp.TCPCandidateAddrs, []string{"203.0.113.20:51100"})
	}

	if vResp.TCPSelectedView != "" || vResp.TCPSelectedReason != "" {
		t.Fatalf("analysis() visitor tcp_selected_view/reason = (%q,%q), want both empty", vResp.TCPSelectedView, vResp.TCPSelectedReason)
	}
	if cResp.TCPSelectedView != "" || cResp.TCPSelectedReason != "" {
		t.Fatalf("analysis() client tcp_selected_view/reason = (%q,%q), want both empty", cResp.TCPSelectedView, cResp.TCPSelectedReason)
	}
}

func TestControllerAnalysis_TCPSelectedViewAffectsTCPCandidateAddrs(t *testing.T) {
	c, err := NewController(time.Minute)
	if err != nil {
		t.Fatalf("NewController() error = %v, want nil", err)
	}

	clientGlobalMapped := []string{"203.0.113.10:41000", "203.0.113.10:41001"}
	visitorGlobalMapped := []string{"203.0.113.20:51000", "203.0.113.20:51001"}

	session := &Session{
		sid: "sid-tcp-selected-view",
		clientMsg: &wire.NatHoleClient{
			TransactionID: "c-tx",
			DirectAddrs:   []string{"[::1]:1"},
			MappedAddrs:   []string{"203.0.113.1:40000", "203.0.113.1:40001"},
			AssistedAddrs: []string{"192.168.0.10:11111"},
			TCPMappedAddrs: []string{
				"198.51.100.1:49999",
			},
			TCPSTUNCN:     &wire.STUNViewObservation{Available: false, NATDifficulty: 999},
			TCPSTUNGlobal: &wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 10, OkCount: 2, MappedAddrs: clientGlobalMapped},
		},
		visitorMsg: &wire.NatHoleVisitor{
			TransactionID: "v-tx",
			DirectAddrs:   []string{"[::1]:2"},
			MappedAddrs:   []string{"203.0.113.2:50000", "203.0.113.2:50001"},
			AssistedAddrs: []string{"192.168.0.20:22222"},
			TCPMappedAddrs: []string{
				"198.51.100.2:59999",
			},
			TCPSTUNCN:     &wire.STUNViewObservation{Available: false, NATDifficulty: 999},
			TCPSTUNGlobal: &wire.STUNViewObservation{Available: true, NATDifficulty: 1, RTTMs: 10, OkCount: 2, MappedAddrs: visitorGlobalMapped},
		},
	}

	vResp, cResp, err := c.analysis(session)
	if err != nil {
		t.Fatalf("analysis() error = %v, want nil", err)
	}

	if vResp.TCPSelectedView != "global" || vResp.TCPSelectedReason != "availability" {
		t.Fatalf("analysis() visitor tcp_selected_view=(%q,%q), want (global,availability)", vResp.TCPSelectedView, vResp.TCPSelectedReason)
	}
	if cResp.TCPSelectedView != "global" || cResp.TCPSelectedReason != "availability" {
		t.Fatalf("analysis() client tcp_selected_view=(%q,%q), want (global,availability)", cResp.TCPSelectedView, cResp.TCPSelectedReason)
	}

	wantClient := []string{"203.0.113.10:41100", "203.0.113.10:41101"}
	if !slices.Equal(vResp.TCPCandidateAddrs, wantClient) {
		t.Fatalf("analysis() visitor TCPCandidateAddrs = %#v, want %#v", vResp.TCPCandidateAddrs, wantClient)
	}
	wantVisitor := []string{"203.0.113.20:51100", "203.0.113.20:51101"}
	if !slices.Equal(cResp.TCPCandidateAddrs, wantVisitor) {
		t.Fatalf("analysis() client TCPCandidateAddrs = %#v, want %#v", cResp.TCPCandidateAddrs, wantVisitor)
	}
}

func TestControllerAnalysis_TCPOnlyRequiresCapability(t *testing.T) {
	c, err := NewController(time.Minute)
	if err != nil {
		t.Fatalf("NewController() error = %v, want nil", err)
	}

	session := &Session{
		sid: "sid-tcp-only-capability",
		clientMsg: &wire.NatHoleClient{
			TransactionID: "c-tx",
			P2PNetwork:    "tcp_only",
			Capabilities:  nil,
		},
		visitorMsg: &wire.NatHoleVisitor{
			TransactionID: "v-tx",
			P2PNetwork:    "tcp_only",
			Capabilities: []string{
				wire.CapabilityTCPP2PV0,
			},
		},
	}

	_, _, err = c.analysis(session)
	if err == nil {
		t.Fatalf("analysis() error = nil, want non-nil")
	}
	if !strings.Contains(err.Error(), "tcp_only requires capability") {
		t.Fatalf("analysis() error = %v, want tcp_only capability error", err)
	}
}
