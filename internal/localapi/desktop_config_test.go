package localapi

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/task"
)

func TestDesktopConfigRouteUpdatesRuntimeConfig(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			MQTTBroker: "broker-old:1883",
		},
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}

	mgr := task.NewManagerWithStatePath(statePath)
	t.Cleanup(mgr.Close)

	srv := NewServer(ListenModeUser, mgr)
	ts := httptest.NewServer(srv.Handler())
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	t.Cleanup(cancel)

	body := bytes.NewBufferString(`{"runtime":{"p2p_network":"udp_only","mqtt_brokers":["broker-a:1883"]},"preferences":{"log_level":"debug"}}`)
	resp := mustDoRequest(t, ctx, http.MethodPatch, ts.URL+"/api/v0/desktop/config", body)
	if resp.StatusCode != http.StatusOK {
		b, _ := io.ReadAll(resp.Body)
		t.Fatalf("PATCH /api/v0/desktop/config status = %d, want %d, body=%s", resp.StatusCode, http.StatusOK, b)
	}
	defer func() { _ = resp.Body.Close() }()

	var snapshot task.DesktopStateSnapshot
	if err := json.NewDecoder(resp.Body).Decode(&snapshot); err != nil {
		t.Fatalf("decode desktop config response: %v", err)
	}
	if got := snapshot.Config.Desired.Runtime.P2PNetwork; got != "udp_only" {
		t.Fatalf("response desired p2p_network = %q, want %q", got, "udp_only")
	}
	if got := snapshot.Config.Desired.Preferences.LogLevel; got != "debug" {
		t.Fatalf("response desired log_level = %q, want %q", got, "debug")
	}
}

func TestDesktopConfigRouteClearsShellPreferences(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			MQTTBroker: "broker-old:1883",
		},
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}

	mgr := task.NewManagerWithStatePath(statePath)
	t.Cleanup(mgr.Close)

	if _, err := mgr.UpdateDesktopConfig(task.DesktopConfigUpdate{
		Preferences: &task.DesktopPreferencesUpdate{
			DefaultShellTarget:  stringPtr("local"),
			DefaultShellSession: stringPtr("main"),
			LogLevel:            "debug",
		},
	}); err != nil {
		t.Fatalf("UpdateDesktopConfig(non-empty preferences) error = %v", err)
	}

	srv := NewServer(ListenModeUser, mgr)
	ts := httptest.NewServer(srv.Handler())
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	t.Cleanup(cancel)

	body := bytes.NewBufferString(`{"preferences":{"default_shell_target":"","default_shell_session":""}}`)
	resp := mustDoRequest(t, ctx, http.MethodPatch, ts.URL+"/api/v0/desktop/config", body)
	if resp.StatusCode != http.StatusOK {
		b, _ := io.ReadAll(resp.Body)
		t.Fatalf("PATCH /api/v0/desktop/config clear shell preferences status = %d, want %d, body=%s", resp.StatusCode, http.StatusOK, b)
	}
	defer func() { _ = resp.Body.Close() }()

	var snapshot task.DesktopStateSnapshot
	if err := json.NewDecoder(resp.Body).Decode(&snapshot); err != nil {
		t.Fatalf("decode desktop config response: %v", err)
	}
	if got := snapshot.Config.Desired.Preferences.DefaultShellTarget; got != "" {
		t.Fatalf("response desired default_shell_target = %q, want empty", got)
	}
	if got := snapshot.Config.Desired.Preferences.DefaultShellSession; got != "" {
		t.Fatalf("response desired default_shell_session = %q, want empty", got)
	}
}

func TestDesktopConfigRouteRejectsInvalidUpdate(t *testing.T) {
	mgr := task.NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	t.Cleanup(mgr.Close)

	srv := NewServer(ListenModeUser, mgr)
	ts := httptest.NewServer(srv.Handler())
	t.Cleanup(ts.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	t.Cleanup(cancel)

	body := bytes.NewBufferString(`{"runtime":{"p2p_network":"bad"}}`)
	resp := mustDoRequest(t, ctx, http.MethodPatch, ts.URL+"/api/v0/desktop/config", body)
	if resp.StatusCode != http.StatusBadRequest {
		b, _ := io.ReadAll(resp.Body)
		t.Fatalf("PATCH invalid desktop config status = %d, want %d, body=%s", resp.StatusCode, http.StatusBadRequest, b)
	}
	defer func() { _ = resp.Body.Close() }()

	var apiErr ErrorResponse
	if err := json.NewDecoder(resp.Body).Decode(&apiErr); err != nil {
		t.Fatalf("decode error response: %v", err)
	}
	if apiErr.ReasonCode != poc.ReasonCodeBadRequest {
		t.Fatalf("error ReasonCode = %q, want %q", apiErr.ReasonCode, poc.ReasonCodeBadRequest)
	}
}

func stringPtr(value string) *string {
	return &value
}
