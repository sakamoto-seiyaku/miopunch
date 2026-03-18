package nathole

import "testing"

func TestClassifyNATFeature(t *testing.T) {
	tests := []struct {
		name      string
		addrs     []string
		localIPs  []string
		wantType  string
		wantBehav string
		wantPub   bool
	}{
		{
			name:      "easy",
			addrs:     []string{"1.1.1.1:1000", "1.1.1.1:1000"},
			wantType:  EasyNAT,
			wantBehav: BehaviorNoChange,
		},
		{
			name:      "port-changed-regular",
			addrs:     []string{"1.1.1.1:1000", "1.1.1.1:1002"},
			wantType:  HardNAT,
			wantBehav: BehaviorPortChanged,
		},
		{
			name:      "ip-changed",
			addrs:     []string{"1.1.1.1:1000", "1.1.1.2:1000"},
			wantType:  HardNAT,
			wantBehav: BehaviorIPChanged,
		},
		{
			name:      "both-changed",
			addrs:     []string{"1.1.1.1:1000", "1.1.1.2:1001"},
			wantType:  HardNAT,
			wantBehav: BehaviorBothChanged,
		},
		{
			name:      "public-network-flag",
			addrs:     []string{"2.2.2.2:1000", "2.2.2.2:1000"},
			localIPs:  []string{"2.2.2.2"},
			wantType:  EasyNAT,
			wantBehav: BehaviorNoChange,
			wantPub:   true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			f, err := ClassifyNATFeature(tt.addrs, tt.localIPs)
			if err != nil {
				t.Fatalf("ClassifyNATFeature: %v", err)
			}
			if f.NatType != tt.wantType || f.Behavior != tt.wantBehav || f.PublicNetwork != tt.wantPub {
				t.Fatalf("got %+v, want type=%s behav=%s pub=%v", *f, tt.wantType, tt.wantBehav, tt.wantPub)
			}
		})
	}
}
