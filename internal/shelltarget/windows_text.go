package shelltarget

import "unicode/utf16"

func decodeWindowsCommandOutput(out []byte) string {
	if !looksLikeUTF16LE(out) {
		return string(out)
	}

	if len(out)%2 != 0 {
		out = out[:len(out)-1]
	}
	u16 := make([]uint16, 0, len(out)/2)
	for i := 0; i+1 < len(out); i += 2 {
		u16 = append(u16, uint16(out[i])|uint16(out[i+1])<<8)
	}
	if len(u16) > 0 && u16[0] == 0xfeff {
		u16 = u16[1:]
	}
	return string(utf16.Decode(u16))
}

func looksLikeUTF16LE(out []byte) bool {
	pairs := 0
	nulHighBytes := 0
	for i := 1; i < len(out); i += 2 {
		pairs++
		if out[i] == 0 {
			nulHighBytes++
		}
	}
	return pairs > 0 && nulHighBytes*2 >= pairs
}
