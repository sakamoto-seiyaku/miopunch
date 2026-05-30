package main

import (
	"testing"

	"github.com/miopunch/miopunch/internal/poc"
)

func TestParseShellArgs(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name          string
		args          []string
		wantPeerID    string
		wantTarget    string
		wantP2P       string
		wantReadyOnly bool
		wantReason    poc.ReasonCode
	}{
		{
			name:       "default target listing",
			args:       []string{"peer-a"},
			wantPeerID: "peer-a",
			wantP2P:    "auto",
		},
		{
			name:          "ready listing",
			args:          []string{"peer-a", "--ready"},
			wantPeerID:    "peer-a",
			wantP2P:       "auto",
			wantReadyOnly: true,
		},
		{
			name:       "target session listing",
			args:       []string{"peer-a", "wsl:Debian"},
			wantPeerID: "peer-a",
			wantTarget: "wsl:Debian",
			wantP2P:    "auto",
		},
		{
			name:       "ready target listing rejected",
			args:       []string{"peer-a", "ssh:ale", "--ready"},
			wantReason: poc.ReasonCodeBadRequest,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotPeerID, gotTarget, gotP2P, gotReadyOnly, failure := parseShellArgs(
				globalOptions{},
				"sh ls",
				tt.args,
				"use: miopunch sh ls <peer_id> [target] [--ready] [-u|-t|--p2p-network ...]",
			)
			if tt.wantReason != "" {
				if failure == nil {
					t.Fatalf("parseShellArgs(%v) failure = nil, want %q", tt.args, tt.wantReason)
				}
				if failure.ReasonCode != tt.wantReason {
					t.Fatalf("parseShellArgs(%v) reasonCode = %q, want %q", tt.args, failure.ReasonCode, tt.wantReason)
				}
				return
			}
			if failure != nil {
				t.Fatalf("parseShellArgs(%v) failure = %#v, want nil", tt.args, failure)
			}
			if gotPeerID != tt.wantPeerID || gotTarget != tt.wantTarget || gotP2P != tt.wantP2P || gotReadyOnly != tt.wantReadyOnly {
				t.Fatalf(
					"parseShellArgs(%v) = (%q, %q, %q, %t), want (%q, %q, %q, %t)",
					tt.args,
					gotPeerID,
					gotTarget,
					gotP2P,
					gotReadyOnly,
					tt.wantPeerID,
					tt.wantTarget,
					tt.wantP2P,
					tt.wantReadyOnly,
				)
			}
		})
	}
}
