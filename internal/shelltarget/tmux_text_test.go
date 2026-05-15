package shelltarget

import "testing"

func TestLooksLikeTmuxMissing(t *testing.T) {
	tests := []struct {
		name string
		out  string
		want bool
	}{
		{
			name: "linux sh",
			out:  "sh: 1: tmux: not found",
			want: true,
		},
		{
			name: "zsh",
			out:  "zsh:1: command not found: tmux",
			want: true,
		},
		{
			name: "windows cmd",
			out:  "'tmux' is not recognized as an internal or external command",
			want: true,
		},
		{
			name: "ssh auth failure",
			out:  "Permission denied (publickey).",
			want: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := looksLikeTmuxMissing(tt.out)
			if got != tt.want {
				t.Errorf("looksLikeTmuxMissing(%q) = %t, want %t", tt.out, got, tt.want)
			}
		})
	}
}

func TestLooksLikeNoTmuxServer(t *testing.T) {
	tests := []struct {
		name string
		out  string
		want bool
	}{
		{
			name: "failed to connect",
			out:  "failed to connect to server",
			want: true,
		},
		{
			name: "no server running",
			out:  "no server running on /tmp/tmux-1000/default",
			want: true,
		},
		{
			name: "tmux 3.1 error connecting",
			out:  "error connecting to /tmp/tmux-1000/default (No such file or directory)",
			want: true,
		},
		{
			name: "ssh auth failure",
			out:  "Permission denied (publickey).",
			want: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := looksLikeNoTmuxServer(tt.out)
			if got != tt.want {
				t.Errorf("looksLikeNoTmuxServer(%q) = %t, want %t", tt.out, got, tt.want)
			}
		})
	}
}

func TestParsePlainTmuxSessionNames(t *testing.T) {
	got := parsePlainTmuxSessionNames([]byte("main\nops\nmain\n\n"))
	want := []string{"main", "ops"}
	if !equalStringSlices(got, want) {
		t.Fatalf("parsePlainTmuxSessionNames(...) = %v, want %v", got, want)
	}
}

func TestParseDefaultTmuxSessionNames(t *testing.T) {
	tests := []struct {
		name string
		out  string
		want []string
	}{
		{
			name: "default tmux output",
			out:  "main: 1 windows (created Fri May 15)\nops: 2 windows\n",
			want: []string{"main", "ops"},
		},
		{
			name: "already clean",
			out:  "main\nops\n",
			want: []string{"main", "ops"},
		},
		{
			name: "duplicates and empty",
			out:  "main: 1 windows\n\nmain: 2 windows\nops: 1 windows\n",
			want: []string{"main", "ops"},
		},
		{
			name: "no server",
			out:  "error connecting to /tmp/tmux-1000/default (No such file or directory)",
			want: []string{},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := parseDefaultTmuxSessionNames([]byte(tt.out))
			if !equalStringSlices(got, tt.want) {
				t.Fatalf("parseDefaultTmuxSessionNames(%q) = %v, want %v", tt.out, got, tt.want)
			}
		})
	}
}

func TestWindowsSSHtmuxCommandArgs(t *testing.T) {
	tests := []struct {
		name string
		got  []string
		want []string
	}{
		{
			name: "list sessions",
			got:  windowsSSHListSessionsArgs("ale"),
			want: []string{"ale", "tmux", "list-sessions", "-F", "#S"},
		},
		{
			name: "preflight",
			got:  windowsSSHPreflightTmuxArgs("ale"),
			want: []string{"ale", "tmux", "-V"},
		},
		{
			name: "attach unchanged",
			got:  windowsSSHAttachArgs("ale", "main"),
			want: []string{"-tt", "ale", "tmux", "new", "-A", "-s", "main"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if !equalStringSlices(tt.got, tt.want) {
				t.Fatalf("%s args = %v, want %v", tt.name, tt.got, tt.want)
			}
			for i, arg := range tt.got {
				if i > 0 && arg == "--" {
					t.Fatalf("%s args = %v, want no remote -- token", tt.name, tt.got)
				}
			}
		})
	}
}

func equalStringSlices(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}
