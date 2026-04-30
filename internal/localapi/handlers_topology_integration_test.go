package localapi

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/task"
)

func TestTopologySnapshot_Redaction_NoSecretsLeaked(t *testing.T) {
	t.Parallel()

	tmp := t.TempDir()
	statePath := filepath.Join(tmp, "state.json")

	// Seed state.json with values that must never be surfaced via topology output.
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			ProxyName:  "self",
			SecretKey:  "SK_SHOULD_NOT_LEAK",
			MQTTBroker: "broker:1883",
		},
		Peers: map[string]pocstate.PeerConfig{
			"PEER_SHOULD_NOT_LEAK": {SecretKey: "PSK_SHOULD_NOT_LEAK"},
		},
	}); err != nil {
		t.Fatalf("save state: %v", err)
	}

	stateDir := filepath.Dir(statePath)

	// Create identity + governance + net + decls on disk so the snapshot has to
	// read from real state files.
	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		t.Fatalf("ensure identity: %v", err)
	}

	secret := make([]byte, 32)
	if _, err := rand.Read(secret); err != nil {
		t.Fatalf("rand net secret: %v", err)
	}
	netID, err := pocstate.NetIDFromSecret(secret)
	if err != nil {
		t.Fatalf("net id from secret: %v", err)
	}
	if err := pocstate.SaveNet(stateDir, pocstate.Net{
		NetSecret:        secret,
		BrokersEffective: []string{"broker:1883"},
	}); err != nil {
		t.Fatalf("save net: %v", err)
	}

	if _, err := pocstate.EnsureGovernanceHeadSnapshot(stateDir, netID, selfID); err != nil {
		t.Fatalf("ensure head snapshot: %v", err)
	}

	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		t.Fatalf("ensure decls: %v", err)
	}

	approveDecl, err := pocstate.NewApproveMemberDeclV0(time.Now().UTC(), selfID, pocstate.ApproveMemberBodyV0{
		MemberPeerID:  selfID.PeerID,
		Ed25519PubB64: selfID.Ed25519PubB64(),
		X25519PubB64:  selfID.X25519PubB64(),
		V4Hint:        "easy",
		V6Hint:        "direct",
	})
	if err != nil {
		t.Fatalf("new approve decl: %v", err)
	}
	if _, err := pocstate.UpdateDecls(stateDir, func(f *pocstate.DeclsFileV0) error {
		f.Decls = append(f.Decls, approveDecl)
		return nil
	}); err != nil {
		t.Fatalf("update decls: %v", err)
	}

	mgr := task.NewManagerWithStatePath(statePath)
	t.Cleanup(mgr.Close)

	srv := NewServer(ListenModeUser, mgr)
	ts := httptest.NewServer(srv.Handler())
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	t.Cleanup(cancel)

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, ts.URL+"/api/v0/topology", nil)
	if err != nil {
		t.Fatalf("new request: %v", err)
	}
	req.Host = poc.LocalAPIHost

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("do request: %v", err)
	}
	t.Cleanup(func() { _ = resp.Body.Close() })

	if resp.StatusCode != http.StatusOK {
		b, _ := io.ReadAll(resp.Body)
		t.Fatalf("GET /api/v0/topology status=%d, want %d, body=%s", resp.StatusCode, http.StatusOK, b)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatalf("read body: %v", err)
	}

	var decoded any
	if err := json.Unmarshal(body, &decoded); err != nil {
		t.Fatalf("unmarshal body: %v", err)
	}

	s := string(body)
	mustNotContain := []string{
		"secret_key",
		"net_secret_b64",
		"invite_code",
		"invite_secret_b64",
		"ed25519_seed_b64",
		"x25519_priv_b64",
		"SK_SHOULD_NOT_LEAK",
		"PSK_SHOULD_NOT_LEAK",
	}
	for _, needle := range mustNotContain {
		if strings.Contains(s, needle) {
			t.Fatalf("topology response leaked %q: body=%s", needle, s)
		}
	}
}
