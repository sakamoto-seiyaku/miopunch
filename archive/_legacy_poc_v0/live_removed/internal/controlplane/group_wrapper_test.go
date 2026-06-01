package controlplane

import (
	"bytes"
	"errors"
	"testing"
)

func TestGroupWrapperV0_RoundTrip(t *testing.T) {
	netSecret := []byte("0123456789abcdef0123456789abcdef")
	plaintext := []byte{0x01, 0x02, 0x03, 0x04}

	frame, err := SealGroupV0(netSecret, plaintext)
	if err != nil {
		t.Fatalf("SealGroupV0() error = %v", err)
	}
	got, err := OpenGroupV0(netSecret, frame)
	if err != nil {
		t.Fatalf("OpenGroupV0() error = %v", err)
	}
	if !bytes.Equal(got, plaintext) {
		t.Fatalf("OpenGroupV0() plaintext = %x, want %x", got, plaintext)
	}
}

func TestGroupWrapperV0_RejectsUnsupportedVersion(t *testing.T) {
	netSecret := []byte("0123456789abcdef0123456789abcdef")
	plaintext := []byte{0x01, 0x02, 0x03}

	frame, err := SealGroupV0(netSecret, plaintext)
	if err != nil {
		t.Fatalf("SealGroupV0() error = %v", err)
	}

	frame[0] = 1
	if err := func() error {
		_, err := OpenGroupV0(netSecret, frame)
		return err
	}(); !errors.Is(err, ErrUnsupportedCiphertextVersion) {
		t.Fatalf("OpenGroupV0(unsupported v) error = %v, want %v", err, ErrUnsupportedCiphertextVersion)
	}
}

func TestGroupWrapperV0_RejectsShortFrame(t *testing.T) {
	netSecret := []byte("0123456789abcdef0123456789abcdef")
	if err := func() error {
		_, err := OpenGroupV0(netSecret, []byte{0})
		return err
	}(); !errors.Is(err, ErrInvalidCiphertextFrame) {
		t.Fatalf("OpenGroupV0(short) error = %v, want %v", err, ErrInvalidCiphertextFrame)
	}
}
