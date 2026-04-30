package pocacceptor

import (
	"strings"
	"testing"
)

func TestSafeStreamMetadataSummaryRedactsSensitiveValues(t *testing.T) {
	got := safeStreamMetadataSummary(map[string]string{
		"op":           "ping",
		"peer_id":      "peer-1",
		"seed_peer":    `{"secret_key":"SECRET_SHOULD_NOT_LEAK"}`,
		"approve_decl": `{"body":"DECL_SHOULD_NOT_LEAK"}`,
		"decls":        `[{"body":"DECLS_SHOULD_NOT_LEAK"}]`,
		"target":       "shell",
		"session":      "main",
	})

	for _, needle := range []string{
		"SECRET_SHOULD_NOT_LEAK",
		"secret_key",
		"DECL_SHOULD_NOT_LEAK",
		"DECLS_SHOULD_NOT_LEAK",
	} {
		if strings.Contains(got, needle) {
			t.Fatalf("safeStreamMetadataSummary(...) = %q, want no %q", got, needle)
		}
	}
	for _, needle := range []string{
		"seed_peer_present=true",
		"approve_decl_present=true",
		"decls_present=true",
		`op="ping"`,
		`peer_id="peer-1"`,
	} {
		if !strings.Contains(got, needle) {
			t.Fatalf("safeStreamMetadataSummary(...) = %q, want %q", got, needle)
		}
	}
}
