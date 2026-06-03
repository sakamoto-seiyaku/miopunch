package udpowner

import "testing"

type recoverableNetError struct {
	timeout   bool
	temporary bool
}

func (e recoverableNetError) Error() string   { return "recoverable net error" }
func (e recoverableNetError) Timeout() bool   { return e.timeout }
func (e recoverableNetError) Temporary() bool { return e.temporary }

func TestRecoverableUDPReadError(t *testing.T) {
	tests := []struct {
		name string
		err  error
		want bool
	}{
		{name: "nil", err: nil, want: false},
		{name: "timeout", err: recoverableNetError{timeout: true}, want: true},
		{name: "temporary", err: recoverableNetError{temporary: true}, want: true},
		{name: "plain", err: errEndpointClosed, want: false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := recoverableUDPReadError(tt.err); got != tt.want {
				t.Fatalf("recoverableUDPReadError(%v) = %t, want %t", tt.err, got, tt.want)
			}
		})
	}
}
