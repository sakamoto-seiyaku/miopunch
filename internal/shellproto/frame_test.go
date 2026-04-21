package shellproto

import (
	"bytes"
	"encoding/binary"
	"errors"
	"io"
	"reflect"
	"strings"
	"testing"
)

func TestFrame_RoundTrip(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name    string
		kind    Kind
		payload []byte
	}{
		{name: "data", kind: KindData, payload: []byte("hello")},
		{name: "json", kind: KindJSON, payload: []byte(`{"k":"v"}`)},
		{name: "empty", kind: KindData, payload: nil},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			var buf bytes.Buffer
			if err := WriteFrame(&buf, tt.kind, tt.payload); err != nil {
				t.Fatalf("WriteFrame() error = %v, want nil", err)
			}

			gotKind, gotPayload, err := ReadFrame(&buf)
			if err != nil {
				t.Fatalf("ReadFrame() error = %v, want nil", err)
			}
			if gotKind != tt.kind {
				t.Fatalf("ReadFrame() kind = %v, want %v", gotKind, tt.kind)
			}
			if !bytes.Equal(gotPayload, tt.payload) {
				t.Fatalf("ReadFrame() payload = %q, want %q", gotPayload, tt.payload)
			}
		})
	}
}

func TestWriteFrame_NilWriter(t *testing.T) {
	t.Parallel()

	if err := WriteFrame(nil, KindData, []byte("x")); err == nil {
		t.Fatalf("WriteFrame(nil, ...) error = nil, want non-nil")
	}
}

func TestReadFrame_NilReader(t *testing.T) {
	t.Parallel()

	if _, _, err := ReadFrame(nil); err == nil {
		t.Fatalf("ReadFrame(nil) error = nil, want non-nil")
	}
}

func TestWriteFrame_TooLarge(t *testing.T) {
	t.Parallel()

	payload := make([]byte, MaxFrameSize+1)
	var buf bytes.Buffer
	if err := WriteFrame(&buf, KindData, payload); err == nil {
		t.Fatalf("WriteFrame(tooLarge) error = nil, want non-nil")
	}
}

func TestReadFrame_TooLargeHeader(t *testing.T) {
	t.Parallel()

	var hdr [5]byte
	hdr[0] = byte(KindData)
	binary.BigEndian.PutUint32(hdr[1:], uint32(MaxFrameSize+1))

	r := bytes.NewReader(hdr[:])
	if _, _, err := ReadFrame(r); err == nil {
		t.Fatalf("ReadFrame(tooLargeHeader) error = nil, want non-nil")
	}
}

func TestWriteJSON_ReadJSON(t *testing.T) {
	t.Parallel()

	want := Control{
		Op:      OpWinSize,
		Target:  "local",
		Session: "main",
		WinSize: &WinSize{Cols: 80, Rows: 24},
	}

	var buf bytes.Buffer
	if err := WriteJSON(&buf, want); err != nil {
		t.Fatalf("WriteJSON() error = %v, want nil", err)
	}

	var got Control
	if err := ReadJSON(&buf, &got); err != nil {
		t.Fatalf("ReadJSON() error = %v, want nil", err)
	}

	if !reflect.DeepEqual(got, want) {
		t.Fatalf("ReadJSON() = %#v, want %#v", got, want)
	}
}

func TestReadJSON_WrongKind(t *testing.T) {
	t.Parallel()

	var buf bytes.Buffer
	if err := WriteFrame(&buf, KindData, []byte("hello")); err != nil {
		t.Fatalf("WriteFrame() error = %v, want nil", err)
	}

	var got Control
	err := ReadJSON(&buf, &got)
	if err == nil {
		t.Fatalf("ReadJSON(wrongKind) error = nil, want non-nil")
	}
	if !strings.Contains(err.Error(), "unexpected frame kind") {
		t.Fatalf("ReadJSON(wrongKind) error = %v, want contains %q", err, "unexpected frame kind")
	}
}

func TestReaderWriter_NilReceiver(t *testing.T) {
	t.Parallel()

	var r *Reader
	if _, _, err := r.ReadFrame(); !errors.Is(err, io.ErrClosedPipe) {
		t.Fatalf("(*Reader)(nil).ReadFrame() error = %v, want %v", err, io.ErrClosedPipe)
	}
	if err := r.ReadJSON(&Control{}); !errors.Is(err, io.ErrClosedPipe) {
		t.Fatalf("(*Reader)(nil).ReadJSON() error = %v, want %v", err, io.ErrClosedPipe)
	}

	var w *Writer
	if err := w.WriteFrame(KindData, []byte("x")); !errors.Is(err, io.ErrClosedPipe) {
		t.Fatalf("(*Writer)(nil).WriteFrame() error = %v, want %v", err, io.ErrClosedPipe)
	}
	if err := w.WriteJSON(Control{Op: OpPing}); !errors.Is(err, io.ErrClosedPipe) {
		t.Fatalf("(*Writer)(nil).WriteJSON() error = %v, want %v", err, io.ErrClosedPipe)
	}
}

