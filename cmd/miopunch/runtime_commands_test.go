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

	usage := "use: miopunch sh ls <peer_id> [target] [--ready] [-u|-t] [-4|-6] [--p2p-network auto|udp_only|tcp_only] [--p2p-ip-family auto|v4|v6]"
	tests := []struct {
		name          string
		args          []string
		wantPeerID    string
		wantTarget    string
		wantP2P       string
		wantFamily    string
		wantReadyOnly bool
		wantReason    poc.ReasonCode
	}{
		{
			name:       "default target listing",
			args:       []string{"peer-a"},
			wantPeerID: "peer-a",
			wantP2P:    "auto",
			wantFamily: "auto",
		},
		{
			name:          "ready listing",
			args:          []string{"peer-a", "--ready"},
			wantPeerID:    "peer-a",
			wantP2P:       "auto",
			wantFamily:    "auto",
			wantReadyOnly: true,
		},
		{
			name:       "target session listing",
			args:       []string{"peer-a", "wsl:Debian"},
			wantPeerID: "peer-a",
			wantTarget: "wsl:Debian",
			wantP2P:    "auto",
			wantFamily: "auto",
		},
		{
			name:       "udp ipv4 listing",
			args:       []string{"peer-a", "-u", "-4"},
			wantPeerID: "peer-a",
			wantP2P:    "udp_only",
			wantFamily: "v4",
		},
		{
			name:       "tcp ipv6 long flags",
			args:       []string{"peer-a", "--p2p-network", "tcp_only", "--p2p-ip-family", "ipv6"},
			wantPeerID: "peer-a",
			wantP2P:    "tcp_only",
			wantFamily: "v6",
		},
		{
			name:       "ready target listing rejected",
			args:       []string{"peer-a", "ssh:ale", "--ready"},
			wantReason: poc.ReasonCodeBadRequest,
		},
		{
			name:       "network conflict rejected",
			args:       []string{"peer-a", "-u", "-t"},
			wantReason: poc.ReasonCodeBadRequest,
		},
		{
			name:       "family conflict rejected",
			args:       []string{"peer-a", "-4", "-6"},
			wantReason: poc.ReasonCodeBadRequest,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotPeerID, gotTarget, gotP2P, gotFamily, gotReadyOnly, failure := parseShellArgs(
				globalOptions{},
				"sh ls",
				tt.args,
				usage,
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
			if gotPeerID != tt.wantPeerID || gotTarget != tt.wantTarget || gotP2P != tt.wantP2P || gotFamily != tt.wantFamily || gotReadyOnly != tt.wantReadyOnly {
				t.Fatalf(
					"parseShellArgs(%v) = (%q, %q, %q, %q, %t), want (%q, %q, %q, %q, %t)",
					tt.args,
					gotPeerID,
					gotTarget,
					gotP2P,
					gotFamily,
					gotReadyOnly,
					tt.wantPeerID,
					tt.wantTarget,
					tt.wantP2P,
					tt.wantFamily,
					tt.wantReadyOnly,
				)
			}
		})
	}
}

func TestParsePeerAndP2PArgs(t *testing.T) {
	t.Parallel()

	usage := "use: miopunch ping <peer_id> [-u|-t] [-4|-6] [--p2p-network auto|udp_only|tcp_only] [--p2p-ip-family auto|v4|v6]"
	tests := []struct {
		name       string
		args       []string
		wantPeerID string
		wantP2P    string
		wantFamily string
		wantReason poc.ReasonCode
	}{
		{
			name:       "default",
			args:       []string{"peer-a"},
			wantPeerID: "peer-a",
			wantP2P:    "auto",
			wantFamily: "auto",
		},
		{
			name:       "udp only",
			args:       []string{"peer-a", "-u"},
			wantPeerID: "peer-a",
			wantP2P:    "udp_only",
			wantFamily: "auto",
		},
		{
			name:       "tcp only",
			args:       []string{"peer-a", "-t"},
			wantPeerID: "peer-a",
			wantP2P:    "tcp_only",
			wantFamily: "auto",
		},
		{
			name:       "ipv4",
			args:       []string{"peer-a", "-4"},
			wantPeerID: "peer-a",
			wantP2P:    "auto",
			wantFamily: "v4",
		},
		{
			name:       "ipv6",
			args:       []string{"peer-a", "-6"},
			wantPeerID: "peer-a",
			wantP2P:    "auto",
			wantFamily: "v6",
		},
		{
			name:       "long family",
			args:       []string{"peer-a", "--p2p-ip-family=ipv4"},
			wantPeerID: "peer-a",
			wantP2P:    "auto",
			wantFamily: "v4",
		},
		{
			name:       "network conflict",
			args:       []string{"peer-a", "-u", "-t"},
			wantReason: poc.ReasonCodeBadRequest,
		},
		{
			name:       "family conflict",
			args:       []string{"peer-a", "-4", "-6"},
			wantReason: poc.ReasonCodeBadRequest,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotPeerID, gotP2P, gotFamily, failure := parsePeerAndP2PArgs(
				globalOptions{},
				"ping",
				tt.args,
				usage,
			)
			if tt.wantReason != "" {
				if failure == nil {
					t.Fatalf("parsePeerAndP2PArgs(%v) failure = nil, want %q", tt.args, tt.wantReason)
				}
				if failure.ReasonCode != tt.wantReason {
					t.Fatalf("parsePeerAndP2PArgs(%v) reasonCode = %q, want %q", tt.args, failure.ReasonCode, tt.wantReason)
				}
				return
			}
			if failure != nil {
				t.Fatalf("parsePeerAndP2PArgs(%v) failure = %#v, want nil", tt.args, failure)
			}
			if gotPeerID != tt.wantPeerID || gotP2P != tt.wantP2P || gotFamily != tt.wantFamily {
				t.Fatalf(
					"parsePeerAndP2PArgs(%v) = (%q, %q, %q), want (%q, %q, %q)",
					tt.args,
					gotPeerID,
					gotP2P,
					gotFamily,
					tt.wantPeerID,
					tt.wantP2P,
					tt.wantFamily,
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
