package event

import (
	"errors"
	"testing"
)

func TestEmitter_EmitReturnsWriterError(t *testing.T) {
	wantErr := errors.New("write failed")
	em := NewEmitter(failingWriter{err: wantErr}, "visitor")

	err := em.Emit(Event{Stage: StageTransport, Kind: KindOK, Name: "transport.payload_exchanged"})
	if !errors.Is(err, wantErr) {
		t.Fatalf("Emitter.Emit() error = %v, want %v", err, wantErr)
	}
}

func TestEmitter_ConvenienceMethodsReturnWriterError(t *testing.T) {
	wantErr := errors.New("write failed")
	em := NewEmitter(failingWriter{err: wantErr}, "client")

	if err := em.Start(StageGather, "start", nil); !errors.Is(err, wantErr) {
		t.Fatalf("Emitter.Start() error = %v, want %v", err, wantErr)
	}
	if err := em.OK(StageGather, "ok", nil); !errors.Is(err, wantErr) {
		t.Fatalf("Emitter.OK() error = %v, want %v", err, wantErr)
	}
	if err := em.Fail(StageGather, wantErr, "fail", nil); !errors.Is(err, wantErr) {
		t.Fatalf("Emitter.Fail() error = %v, want %v", err, wantErr)
	}
}

func TestEmitter_IgnoredErrorDoesNotPanic(t *testing.T) {
	em := NewEmitter(failingWriter{err: errors.New("write failed")}, "visitor")

	em.Emit(Event{Stage: StageAttempt, Kind: KindInfo, Name: "ignored"})
	em.OK(StageAttempt, "ignored", nil)
}

type failingWriter struct {
	err error
}

func (w failingWriter) Write(_ []byte) (int, error) {
	return 0, w.err
}
