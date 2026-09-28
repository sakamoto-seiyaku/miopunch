package connectivity

import "testing"

func TestParseP2PNetwork(t *testing.T) {
	t.Parallel()

	tests := []struct {
		in      string
		want    P2PNetwork
		wantErr bool
	}{
		{in: "", want: P2PNetworkAuto},
		{in: "auto", want: P2PNetworkAuto},
		{in: "u", want: P2PNetworkUDPOnly},
		{in: "udp", want: P2PNetworkUDPOnly},
		{in: "udp_only", want: P2PNetworkUDPOnly},
		{in: "t", want: P2PNetworkTCPOnly},
		{in: "tcp", want: P2PNetworkTCPOnly},
		{in: "tcp_only", want: P2PNetworkTCPOnly},
		{in: "bogus", wantErr: true},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.in, func(t *testing.T) {
			t.Parallel()
			got, err := ParseP2PNetwork(tt.in)
			if (err != nil) != tt.wantErr {
				t.Fatalf("ParseP2PNetwork(%q) error = %v, wantErr=%v", tt.in, err, tt.wantErr)
			}
			if tt.wantErr {
				return
			}
			if got != tt.want {
				t.Fatalf("ParseP2PNetwork(%q) = %q, want %q", tt.in, got, tt.want)
			}
		})
	}
}

func TestMergeP2PNetwork(t *testing.T) {
	t.Parallel()

	got, err := MergeP2PNetwork(P2PNetworkAuto, P2PNetworkTCPOnly)
	if err != nil {
		t.Fatalf("MergeP2PNetwork() error = %v, want nil", err)
	}
	if got != P2PNetworkTCPOnly {
		t.Fatalf("MergeP2PNetwork() = %q, want %q", got, P2PNetworkTCPOnly)
	}

	if _, err := MergeP2PNetwork(P2PNetworkUDPOnly, P2PNetworkTCPOnly); err == nil {
		t.Fatalf("expected mismatch error")
	}
}
