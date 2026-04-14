package connectivity

import "testing"

func TestSanitizeMappedAddrs(t *testing.T) {
	t.Parallel()

	got, dropped := sanitizeMappedAddrs([]string{
		"203.0.113.1:40000",
		"",
		"   ",
		"203.0.113.2",
		"203.0.113.3:40001",
	})

	if len(got) != 2 || got[0] != "203.0.113.1:40000" || got[1] != "203.0.113.3:40001" {
		t.Fatalf("sanitizeMappedAddrs() valid = %#v, want two valid host:port entries", got)
	}
	if len(dropped) != 3 || dropped[0] != "<empty>" || dropped[1] != "<empty>" || dropped[2] != "203.0.113.2" {
		t.Fatalf("sanitizeMappedAddrs() dropped = %#v, want empty markers and invalid host:port", dropped)
	}
}
