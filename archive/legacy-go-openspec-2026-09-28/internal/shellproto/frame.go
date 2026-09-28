package shellproto

import (
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
)

type Kind uint8

const (
	KindData Kind = 0
	KindJSON Kind = 1
)

const (
	// MaxFrameSize is a defensive upper bound for a single frame payload.
	MaxFrameSize = 4 << 20
)

func WriteFrame(w io.Writer, kind Kind, payload []byte) error {
	if w == nil {
		return errors.New("nil writer")
	}
	if len(payload) > MaxFrameSize {
		return fmt.Errorf("frame too large: %d", len(payload))
	}

	var hdr [5]byte
	hdr[0] = byte(kind)
	binary.BigEndian.PutUint32(hdr[1:], uint32(len(payload)))
	if _, err := w.Write(hdr[:]); err != nil {
		return err
	}
	_, err := w.Write(payload)
	return err
}

func ReadFrame(r io.Reader) (Kind, []byte, error) {
	if r == nil {
		return 0, nil, errors.New("nil reader")
	}

	var hdr [5]byte
	if _, err := io.ReadFull(r, hdr[:]); err != nil {
		return 0, nil, err
	}
	kind := Kind(hdr[0])
	n32 := binary.BigEndian.Uint32(hdr[1:])
	if n32 > MaxFrameSize {
		return 0, nil, fmt.Errorf("frame too large: %d", n32)
	}
	n := int(n32)
	buf := make([]byte, n)
	if _, err := io.ReadFull(r, buf); err != nil {
		return 0, nil, err
	}
	return kind, buf, nil
}

func WriteJSON(w io.Writer, v any) error {
	data, err := json.Marshal(v)
	if err != nil {
		return err
	}
	return WriteFrame(w, KindJSON, data)
}

func ReadJSON(r io.Reader, out any) error {
	kind, payload, err := ReadFrame(r)
	if err != nil {
		return err
	}
	if kind != KindJSON {
		return fmt.Errorf("unexpected frame kind: %d", kind)
	}
	return json.Unmarshal(payload, out)
}
