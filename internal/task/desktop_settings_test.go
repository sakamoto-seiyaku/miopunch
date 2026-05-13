package task

import (
	"errors"
	"path/filepath"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/pocstate"
)

func TestUpdateDesktopConfigPersistsRuntimeAndPreferences(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			MQTTBroker:  "broker-old:1883",
			DataProto:   "quic",
			QUICCC:      "bbr",
			P2PNetwork:  "auto",
			P2PIPFamily: "auto",
		},
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	disablePortMap := true
	snapshot, err := m.UpdateDesktopConfig(DesktopConfigUpdate{
		Runtime: &DesktopRuntimeConfigUpdate{
			MQTTBrokers:    []string{"broker-a:1883", "broker-b:1883"},
			P2PNetwork:     "tcp_only",
			P2PIPFamily:    "v4",
			DataProto:      "kcp",
			QUICCC:         "brutal",
			StunServers:    []string{"stun-a:3478"},
			DisablePortMap: &disablePortMap,
		},
		Preferences: &DesktopPreferencesUpdate{
			DefaultShellTarget:  stringPtr("local"),
			DefaultShellSession: stringPtr("main"),
			LogLevel:            "debug",
		},
	})
	if err != nil {
		t.Fatalf("UpdateDesktopConfig() error = %v", err)
	}

	if got := snapshot.Config.Desired.Runtime.P2PNetwork; got != "tcp_only" {
		t.Errorf("UpdateDesktopConfig().Config.Desired.Runtime.P2PNetwork = %q, want %q", got, "tcp_only")
	}
	if got := snapshot.Config.Desired.Preferences.LogLevel; got != "debug" {
		t.Errorf("UpdateDesktopConfig().Config.Desired.Preferences.LogLevel = %q, want %q", got, "debug")
	}
	if !snapshot.Config.Desired.Runtime.DisablePortMap {
		t.Error("UpdateDesktopConfig().Config.Desired.Runtime.DisablePortMap = false, want true")
	}

	st, err := pocstate.Load(statePath)
	if err != nil {
		t.Fatalf("pocstate.Load(%q) error = %v", statePath, err)
	}
	if got := st.Local.MQTTBrokerEndpoints(); len(got) != 2 || got[0] != "broker-a:1883" || got[1] != "broker-b:1883" {
		t.Fatalf("persisted MQTTBrokerEndpoints = %v, want [broker-a:1883 broker-b:1883]", got)
	}
	if got := st.Local.DataProto; got != "kcp" {
		t.Errorf("persisted DataProto = %q, want %q", got, "kcp")
	}

	settings, err := m.loadDesktopSettings()
	if err != nil {
		t.Fatalf("loadDesktopSettings() error = %v", err)
	}
	if got := settings.Preferences.DefaultShellTarget; got != "local" {
		t.Errorf("desktop settings DefaultShellTarget = %q, want %q", got, "local")
	}
	if got := settings.Preferences.LogLevel; got != "debug" {
		t.Errorf("desktop settings LogLevel = %q, want %q", got, "debug")
	}
}

func TestUpdateDesktopConfigClearsShellPreferences(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			MQTTBroker: "broker-old:1883",
		},
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	if _, err := m.UpdateDesktopConfig(DesktopConfigUpdate{
		Preferences: &DesktopPreferencesUpdate{
			DefaultShellTarget:  stringPtr("local"),
			DefaultShellSession: stringPtr("main"),
			LogLevel:            "debug",
		},
	}); err != nil {
		t.Fatalf("UpdateDesktopConfig(non-empty preferences) error = %v", err)
	}

	snapshot, err := m.UpdateDesktopConfig(DesktopConfigUpdate{
		Preferences: &DesktopPreferencesUpdate{
			DefaultShellTarget:  stringPtr(""),
			DefaultShellSession: stringPtr(""),
		},
	})
	if err != nil {
		t.Fatalf("UpdateDesktopConfig(clear shell preferences) error = %v", err)
	}
	if got := snapshot.Config.Desired.Preferences.DefaultShellTarget; got != "" {
		t.Errorf("UpdateDesktopConfig(clear).Config.Desired.Preferences.DefaultShellTarget = %q, want empty", got)
	}
	if got := snapshot.Config.Desired.Preferences.DefaultShellSession; got != "" {
		t.Errorf("UpdateDesktopConfig(clear).Config.Desired.Preferences.DefaultShellSession = %q, want empty", got)
	}

	settings, err := m.loadDesktopSettings()
	if err != nil {
		t.Fatalf("loadDesktopSettings() error = %v", err)
	}
	if got := settings.Preferences.DefaultShellTarget; got != "" {
		t.Errorf("desktop settings DefaultShellTarget = %q, want empty", got)
	}
	if got := settings.Preferences.DefaultShellSession; got != "" {
		t.Errorf("desktop settings DefaultShellSession = %q, want empty", got)
	}
}

func TestUpdateDesktopConfigRejectsInvalidP2PNetworkWithoutSaving(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			MQTTBroker: "broker-old:1883",
		},
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	_, err := m.UpdateDesktopConfig(DesktopConfigUpdate{
		Runtime: &DesktopRuntimeConfigUpdate{
			P2PNetwork: "invalid",
		},
	})
	var validationErr *DesktopConfigValidationError
	if !errors.As(err, &validationErr) {
		t.Fatalf("UpdateDesktopConfig() error = %v, want *DesktopConfigValidationError", err)
	}

	st, err := pocstate.Load(statePath)
	if err != nil {
		t.Fatalf("pocstate.Load(%q) error = %v", statePath, err)
	}
	if got := st.Local.P2PNetwork; got != "" {
		t.Fatalf("persisted P2PNetwork = %q, want unchanged empty value", got)
	}
}

func stringPtr(value string) *string {
	return &value
}

func TestUpdateDesktopConfigPublishesConfigAfterPreferencesPersist(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			MQTTBroker: "broker-old:1883",
		},
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	sub, _, err := m.SubscribeDesktopStateWithSnapshot()
	if err != nil {
		t.Fatalf("SubscribeDesktopStateWithSnapshot() error = %v", err)
	}
	t.Cleanup(sub.Close)

	configCh := make(chan DesktopStateEvent, 1)
	done := make(chan struct{})
	go func() {
		defer close(done)
		for ev := range sub.C {
			if ev.Kind == DesktopStateEventConfigReplace {
				configCh <- ev
				return
			}
		}
	}()

	if _, err := m.UpdateDesktopConfig(DesktopConfigUpdate{
		Preferences: &DesktopPreferencesUpdate{
			LogLevel: "debug",
		},
	}); err != nil {
		t.Fatalf("UpdateDesktopConfig() error = %v", err)
	}

	var ev DesktopStateEvent
	select {
	case ev = <-configCh:
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for desktop state event kind %q", DesktopStateEventConfigReplace)
	}
	<-done
	if ev.Config == nil {
		t.Fatalf("config.replace Config = nil, want config")
	}
	if got := ev.Config.Desired.Preferences.LogLevel; got != "debug" {
		t.Fatalf("config.replace desired log_level = %q, want %q", got, "debug")
	}
	if got := ev.Config.Effective.Preferences.LogLevel; got != "debug" {
		t.Fatalf("config.replace effective log_level = %q, want %q", got, "debug")
	}
}

func TestUpdateDesktopConfigRejectedUpdateDoesNotPublishConfig(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	if err := pocstate.Save(statePath, pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			MQTTBroker: "broker-old:1883",
		},
	}); err != nil {
		t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
	}

	m := NewManagerWithStatePath(statePath)
	t.Cleanup(m.Close)

	sub, _, err := m.SubscribeDesktopStateWithSnapshot()
	if err != nil {
		t.Fatalf("SubscribeDesktopStateWithSnapshot() error = %v", err)
	}
	t.Cleanup(sub.Close)

	_, err = m.UpdateDesktopConfig(DesktopConfigUpdate{
		Runtime: &DesktopRuntimeConfigUpdate{
			P2PNetwork: "invalid",
		},
	})
	var validationErr *DesktopConfigValidationError
	if !errors.As(err, &validationErr) {
		t.Fatalf("UpdateDesktopConfig() error = %v, want *DesktopConfigValidationError", err)
	}

	select {
	case ev := <-sub.C:
		t.Fatalf("unexpected desktop event after rejected update: %+v", ev)
	case <-time.After(100 * time.Millisecond):
	}
}

func TestUpdateDesktopConfigRejectsInvalidEndpointsWithoutSaving(t *testing.T) {
	tests := []struct {
		name   string
		update DesktopConfigUpdate
	}{
		{
			name: "mqtt broker missing port",
			update: DesktopConfigUpdate{
				Runtime: &DesktopRuntimeConfigUpdate{
					MQTTBrokers: []string{"broker-without-port"},
				},
			},
		},
		{
			name: "stun unsupported scheme",
			update: DesktopConfigUpdate{
				Runtime: &DesktopRuntimeConfigUpdate{
					StunServers: []string{"https://stun.example:3478"},
				},
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			statePath := filepath.Join(t.TempDir(), "state.json")
			if err := pocstate.Save(statePath, pocstate.State{
				Format: pocstate.FormatV0,
				Local: &pocstate.LocalConfig{
					MQTTBroker:  "broker-old:1883",
					StunServers: []string{"stun-old:3478"},
				},
			}); err != nil {
				t.Fatalf("pocstate.Save(%q) error = %v", statePath, err)
			}

			m := NewManagerWithStatePath(statePath)
			t.Cleanup(m.Close)

			_, err := m.UpdateDesktopConfig(tt.update)
			var validationErr *DesktopConfigValidationError
			if !errors.As(err, &validationErr) {
				t.Fatalf("UpdateDesktopConfig(%s) error = %v, want *DesktopConfigValidationError", tt.name, err)
			}

			st, err := pocstate.Load(statePath)
			if err != nil {
				t.Fatalf("pocstate.Load(%q) error = %v", statePath, err)
			}
			if got := st.Local.MQTTBrokerEndpoints(); len(got) != 1 || got[0] != "broker-old:1883" {
				t.Fatalf("persisted MQTTBrokerEndpoints = %v, want [broker-old:1883]", got)
			}
			if got := st.Local.StunServers; len(got) != 1 || got[0] != "stun-old:3478" {
				t.Fatalf("persisted StunServers = %v, want [stun-old:3478]", got)
			}
		})
	}
}
