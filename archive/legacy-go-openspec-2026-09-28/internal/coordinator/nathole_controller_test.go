package coordinator

import (
	"errors"
	"testing"
	"time"
)

func TestControllerGenSid_RandIDFailureFailsClosed(t *testing.T) {
	c, err := NewController(time.Minute)
	if err != nil {
		t.Fatalf("NewController() error = %v, want nil", err)
	}

	wantErr := errors.New("entropy unavailable")
	origRandID := randID
	randID = func() (string, error) {
		return "", wantErr
	}
	t.Cleanup(func() { randID = origRandID })

	got, err := c.GenSid()
	if !errors.Is(err, wantErr) {
		t.Fatalf("Controller.GenSid() error = %v, want %v", err, wantErr)
	}
	if got != "" {
		t.Fatalf("Controller.GenSid() sid = %q, want empty on error", got)
	}
}
