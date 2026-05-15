package shelltarget

import "testing"

func TestDecodeWindowsCommandOutputUTF16LE(t *testing.T) {
	in := []byte{
		'U', 0, 'b', 0, 'u', 0, 'n', 0, 't', 0, 'u', 0, '\r', 0, '\n', 0,
		'D', 0, 'e', 0, 'b', 0, 'i', 0, 'a', 0, 'n', 0, '\r', 0, '\n', 0,
	}

	got := decodeWindowsCommandOutput(in)
	want := "Ubuntu\r\nDebian\r\n"
	if got != want {
		t.Fatalf("decodeWindowsCommandOutput(%v) = %q, want %q", in, got, want)
	}
}

func TestDecodeWindowsCommandOutputUTF8(t *testing.T) {
	in := []byte("Ubuntu\r\nDebian\r\n")

	got := decodeWindowsCommandOutput(in)
	want := "Ubuntu\r\nDebian\r\n"
	if got != want {
		t.Fatalf("decodeWindowsCommandOutput(%q) = %q, want %q", string(in), got, want)
	}
}
