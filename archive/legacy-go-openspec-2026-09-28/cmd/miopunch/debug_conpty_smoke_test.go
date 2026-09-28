package main

import (
	"bytes"
	"reflect"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/shelltarget"
)

func TestDecodeConPTYSmokeInput(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name  string
		value string
		want  []byte
	}{
		{name: "empty"},
		{name: "carriage return", value: `\r`, want: []byte{'\r'}},
		{name: "escaped text", value: `echo hi\r\n`, want: []byte("echo hi\r\n")},
		{name: "escape byte", value: `\x1b[A`, want: []byte{0x1b, '[', 'A'}},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			got, err := decodeConPTYSmokeInput(tt.value)
			if err != nil {
				t.Fatalf("decodeConPTYSmokeInput(%q) error = %v, want nil", tt.value, err)
			}
			if !bytes.Equal(got, tt.want) {
				t.Errorf("decodeConPTYSmokeInput(%q) = %q, want %q", tt.value, got, tt.want)
			}
		})
	}
}

func TestDebugConPTYSmokeRequest(t *testing.T) {
	t.Parallel()

	timeout := 7 * time.Second
	writeDelay := 150 * time.Millisecond
	tests := []struct {
		name       string
		args       []string
		writeInput string
		wantReq    shelltarget.ConPTYSmokeRequest
		wantLabel  string
	}{
		{
			name:      "cmd",
			args:      []string{"cmd"},
			wantLabel: "cmd",
			wantReq: shelltarget.ConPTYSmokeRequest{
				Application: "cmd.exe",
				Args:        []string{"/d", "/c", "echo __MIO_CONPTY_CMD__"},
				Timeout:     timeout,
				WriteDelay:  writeDelay,
				Cols:        132,
				Rows:        43,
			},
		},
		{
			name:      "ssh tmux default input",
			args:      []string{"ssh-tmux", "ale", "main"},
			wantLabel: "ssh-tmux",
			wantReq: shelltarget.ConPTYSmokeRequest{
				Application: "ssh",
				Args:        []string{"-tt", "ale", "tmux", "new", "-A", "-s", "main"},
				Input:       []byte{'\r'},
				Timeout:     timeout,
				WriteDelay:  writeDelay,
				Cols:        132,
				Rows:        43,
			},
		},
		{
			name:       "raw with explicit input",
			args:       []string{"raw", "ssh", "-tt", "ale"},
			writeInput: `\x1b[A`,
			wantLabel:  "raw",
			wantReq: shelltarget.ConPTYSmokeRequest{
				Application: "ssh",
				Args:        []string{"-tt", "ale"},
				Input:       []byte{0x1b, '[', 'A'},
				Timeout:     timeout,
				WriteDelay:  writeDelay,
				Cols:        132,
				Rows:        43,
			},
		},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			gotReq, gotLabel, err := debugConPTYSmokeRequest(
				tt.args,
				timeout,
				writeDelay,
				tt.writeInput,
				132,
				43,
			)
			if err != nil {
				t.Fatalf("debugConPTYSmokeRequest(%q) error = %v, want nil", tt.args, err)
			}
			if gotLabel != tt.wantLabel {
				t.Errorf("debugConPTYSmokeRequest(%q) label = %q, want %q", tt.args, gotLabel, tt.wantLabel)
			}
			if !reflect.DeepEqual(gotReq, tt.wantReq) {
				t.Errorf("debugConPTYSmokeRequest(%q) request = %#v, want %#v", tt.args, gotReq, tt.wantReq)
			}
		})
	}
}
