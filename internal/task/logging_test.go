package task

import (
	"strings"
	"testing"
)

func TestRedactTaskLogString(t *testing.T) {
	in := strings.Join([]string{
		"invite_code=invitevalue",
		"secret_key=keyvalue",
		"net_secret_b64=netvalue",
		"invite_secret_b64=invitekeyvalue",
		"peer_id=peer-ok",
	}, " ")

	got := redactTaskLogString(in)

	for _, secret := range []string{"invitevalue", "keyvalue", "netvalue", "invitekeyvalue"} {
		if strings.Contains(got, secret) {
			t.Fatalf("redactTaskLogString(%q) = %q, want secret %q redacted", in, got, secret)
		}
	}
	if !strings.Contains(got, "peer_id=peer-ok") {
		t.Fatalf("redactTaskLogString(%q) = %q, want non-secret peer_id preserved", in, got)
	}
}
