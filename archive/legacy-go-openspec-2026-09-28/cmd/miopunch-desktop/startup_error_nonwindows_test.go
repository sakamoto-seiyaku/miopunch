//go:build desktop && !windows

package main

import (
	"errors"
	"testing"
)

func TestIsLinuxGTKStartupError(t *testing.T) {
	tests := []struct {
		name string
		err  error
		want bool
	}{
		{
			name: "gtk panic",
			err:  errors.New("desktop runtime panic: failed to init GTK"),
			want: true,
		},
		{
			name: "missing display",
			err:  errors.New("no Linux desktop display found: DISPLAY and WAYLAND_DISPLAY are empty"),
			want: true,
		},
		{
			name: "other error",
			err:  errors.New("desktop runtime failed: other"),
			want: false,
		},
		{
			name: "nil",
			err:  nil,
			want: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := isLinuxGTKStartupError(tt.err)
			if got != tt.want {
				t.Fatalf("isLinuxGTKStartupError(%v) = %t, want %t", tt.err, got, tt.want)
			}
		})
	}
}
