package punchdecision

import (
	"slices"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/punching"
)

func TestAnalyzer_GetRecommandBehaviors_ReturnsSupportedRoleAndMode(t *testing.T) {
	a := NewAnalyzer(time.Minute)

	c := &NatFeature{NatType: EasyNAT}
	v := &NatFeature{NatType: HardNAT, RegularPortsChange: true}

	mode, index, cb, vb := a.GetRecommandBehaviors("k", c, v)
	if !slices.Contains(punching.SupportedModes, mode) {
		t.Fatalf("unsupported mode: %d", mode)
	}
	if index < 0 {
		t.Fatalf("invalid index: %d", index)
	}
	if !slices.Contains(punching.SupportedRoles, cb.Role) || !slices.Contains(punching.SupportedRoles, vb.Role) {
		t.Fatalf("unsupported roles: client=%q visitor=%q", cb.Role, vb.Role)
	}

	a.ReportSuccess("k", mode, index)
}
