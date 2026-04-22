package http_panel

import (
	"errors"
	"strings"
	"testing"
)

func TestListen_LoopbackOnly(t *testing.T) {
	t.Parallel()

	cases := []struct {
		name                string
		addr                string
		wantErr             bool
		wantProblemContains string
	}{
		{
			name:                "reject_wildcard",
			addr:                "0.0.0.0:27400",
			wantErr:             true,
			wantProblemContains: "loopback-only",
		},
		{
			name:                "reject_localhost",
			addr:                "localhost:27400",
			wantErr:             true,
			wantProblemContains: "loopback-only",
		},
		{
			name:                "reject_missing_port",
			addr:                "127.0.0.1",
			wantErr:             true,
			wantProblemContains: "missing port in address",
		},
		{
			name:    "accept_ephemeral_port",
			addr:    "127.0.0.1:0",
			wantErr: false,
		},
		{
			name:    "accept_trimmed_addr",
			addr:    " 127.0.0.1:0 ",
			wantErr: false,
		},
	}

	for _, tc := range cases {
		tc := tc
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()

			ln, gotAddr, err := Listen(tc.addr)
			if tc.wantErr {
				if err == nil {
					if ln != nil {
						_ = ln.Close()
					}
					t.Fatalf("Listen(%q) error=nil, want non-nil", tc.addr)
				}
				if ln != nil {
					_ = ln.Close()
					t.Fatalf("Listen(%q) listener non-nil on error", tc.addr)
				}
				if gotAddr != "" {
					t.Fatalf("Listen(%q) addr=%q, want empty on error", tc.addr, gotAddr)
				}

				var addrErr *ListenAddrError
				if !errors.As(err, &addrErr) {
					t.Fatalf("Listen(%q) error type=%T, want *ListenAddrError", tc.addr, err)
				}
				if tc.wantProblemContains != "" && !strings.Contains(addrErr.Problem, tc.wantProblemContains) {
					t.Fatalf("Listen(%q) problem=%q, want contains %q", tc.addr, addrErr.Problem, tc.wantProblemContains)
				}
				return
			}

			if err != nil {
				if ln != nil {
					_ = ln.Close()
				}
				t.Fatalf("Listen(%q) error=%v, want nil", tc.addr, err)
			}
			if ln == nil {
				t.Fatalf("Listen(%q) listener=nil, want non-nil", tc.addr)
			}
			t.Cleanup(func() { _ = ln.Close() })

			if gotAddr == "" {
				t.Fatalf("Listen(%q) addr empty, want non-empty", tc.addr)
			}
			if !strings.HasPrefix(gotAddr, "127.0.0.1:") {
				t.Fatalf("Listen(%q) addr=%q, want prefix %q", tc.addr, gotAddr, "127.0.0.1:")
			}
		})
	}
}
