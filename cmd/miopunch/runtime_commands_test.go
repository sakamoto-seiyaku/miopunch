package main

import (
	"bytes"
	"encoding/json"
	"testing"

	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
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

func TestExitWithActionSuccessJSONPreservesSelectedPathFact(t *testing.T) {
	t.Parallel()

	var stdout, stderr bytes.Buffer
	exitCode := exitWithActionSuccess(
		globalOptions{Format: outputFormatJSON},
		&stdout,
		&stderr,
		"ping",
		pocruntime.ActionResult{
			Stage:      pocruntime.StageShell,
			ReasonCode: poc.ReasonCodeOK,
			ExitCode:   poc.ExitCodeOK,
			Evidence: pocruntime.Evidence{
				Facts: []poc.Fact{{Message: "selected_path=direct_ipv4"}},
			},
		},
	)
	if exitCode != 0 {
		t.Fatalf("exitWithActionSuccess() exitCode = %d, want 0", exitCode)
	}
	if stderr.Len() != 0 {
		t.Fatalf("exitWithActionSuccess() stderr = %q, want empty", stderr.String())
	}
	var env poc.EnvelopeJSONV0
	if err := json.Unmarshal(stdout.Bytes(), &env); err != nil {
		t.Fatalf("json.Unmarshal(stdout) error = %v, stdout=%q", err, stdout.String())
	}
	if len(env.Facts) != 1 || env.Facts[0].Message != "selected_path=direct_ipv4" {
		t.Fatalf("exitWithActionSuccess() facts = %#v, want selected_path fact", env.Facts)
	}
}
