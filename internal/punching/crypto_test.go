package punching

import (
	"errors"
	"testing"

	"github.com/miopunch/miopunch/internal/wire"
)

func TestEncodeMessage_PrefixesPunchTag(t *testing.T) {
	key := []byte("test-key-32-bytes-likely-enough")
	msg := &wire.NatHoleSid{
		TransactionID: "tx",
		Sid:           "sid",
		Response:      true,
	}

	data, err := EncodeMessage(msg, key)
	if err != nil {
		t.Fatalf("EncodeMessage error: %v", err)
	}
	if !HasPunchTag(data) {
		t.Fatalf("expected PunchTagV1 prefix, got %x", data[:min(len(data), 16)])
	}

	var out wire.NatHoleSid
	if err := DecodeMessageInto(data, key, &out); err != nil {
		t.Fatalf("DecodeMessageInto error: %v", err)
	}
	if out.Sid != msg.Sid || out.TransactionID != msg.TransactionID || out.Response != msg.Response {
		t.Fatalf("decoded mismatch: %#v vs %#v", out, *msg)
	}
}

func TestDecodeMessageInto_RequiresPunchTag(t *testing.T) {
	key := []byte("test-key")
	var out wire.NatHoleSid

	// Non-tagged payload should be classified without attempting decrypt.
	err := DecodeMessageInto([]byte("not-tagged"), key, &out)
	if !errors.Is(err, ErrNotPunchPacket) {
		t.Fatalf("expected ErrNotPunchPacket, got %v", err)
	}

	// Wrong tag / version should also be treated as non-punching.
	bad := append([]byte(nil), PunchTagV1...)
	bad[len(bad)-1] = 0xFF
	bad = append(bad, []byte("ciphertext")...)
	err = DecodeMessageInto(bad, key, &out)
	if !errors.Is(err, ErrNotPunchPacket) {
		t.Fatalf("expected ErrNotPunchPacket for bad tag, got %v", err)
	}
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
