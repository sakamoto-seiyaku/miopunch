package coordinator

import (
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
