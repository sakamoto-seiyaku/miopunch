package punchwire

import (
	"bytes"
	"errors"

	"github.com/fatedier/golib/crypto"

	"github.com/miopunch/miopunch/internal/wire"
)

var (
	// PunchTagV1 is a fixed prefix for all UDP traversal packets (direct handshake
	// and punching messages). It serves two purposes:
	// 1) Stable demux from QUIC via Transport.ReadNonQUICPacket (first byte is 0x00).
	// 2) Avoid feeding late traversal traffic into KCP accept paths.
	PunchTagV1 = []byte{0x00, 'M', 'P', 0x00, 0x01}

	// ErrNotPunchPacket indicates the datagram doesn't carry a tag-prefixed
	// miopunch traversal payload.
	ErrNotPunchPacket = errors.New("not a tagged punching packet")
)

// HasPunchTag reports whether the datagram is a tagged miopunch traversal packet.
func HasPunchTag(b []byte) bool {
	return len(b) >= len(PunchTagV1) && bytes.Equal(b[:len(PunchTagV1)], PunchTagV1)
}

func EncodeMessage(m wire.Message, key []byte) ([]byte, error) {
	buffer := bytes.NewBuffer(nil)
	if err := wire.WriteMsg(buffer, m); err != nil {
		return nil, err
	}
	enc, err := crypto.Encode(buffer.Bytes(), key)
	if err != nil {
		return nil, err
	}
	out := make([]byte, 0, len(PunchTagV1)+len(enc))
	out = append(out, PunchTagV1...)
	out = append(out, enc...)
	return out, nil
}

func DecodeMessageInto(data, key []byte, m wire.Message) error {
	if !HasPunchTag(data) {
		return ErrNotPunchPacket
	}
	// golib/crypto.Decode decrypts in-place, so we must not pass a slice backed
	// by a shared buffer that callers might still need (e.g. a socket owner demux
	// that forwards the raw datagram after peeking TransactionID).
	enc := make([]byte, len(data)-len(PunchTagV1))
	copy(enc, data[len(PunchTagV1):])
	buf, err := crypto.Decode(enc, key)
	if err != nil {
		return err
	}
	return wire.ReadMsgInto(bytes.NewReader(buf), m)
}
