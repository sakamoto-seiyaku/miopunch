package task

import (
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/miopunch/miopunch/internal/atomicfile"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/stunclient"
)

const (
	desktopSettingsFormatV0 = "miopunch.desktop_settings.v0"
	defaultDesktopLogLevel  = "info"
)

// DesktopRuntimeConfigUpdate is a partial update for daemon runtime settings.
type DesktopRuntimeConfigUpdate struct {
	MQTTBrokers          []string `json:"mqtt_brokers,omitempty"`
	P2PNetwork           string   `json:"p2p_network,omitempty"`
	P2PIPFamily          string   `json:"p2p_ip_family,omitempty"`
	DataProto            string   `json:"data_proto,omitempty"`
	QUICCC               string   `json:"quic_cc,omitempty"`
	StunServers          []string `json:"stun,omitempty"`
	StunExplicit         *bool    `json:"stun_explicit,omitempty"`
	DisablePortMap       *bool    `json:"disable_portmap,omitempty"`
	DisableAssistedAddrs *bool    `json:"disable_assisted_addrs,omitempty"`
}

// DesktopPreferencesUpdate is a partial update for desktop-only preferences.
type DesktopPreferencesUpdate struct {
	DefaultShellTarget  *string `json:"default_shell_target,omitempty"`
	DefaultShellSession *string `json:"default_shell_session,omitempty"`
	LogLevel            string  `json:"log_level,omitempty"`
}

// DesktopConfigUpdate is the request body for desktop Settings saves.
type DesktopConfigUpdate struct {
	Runtime     *DesktopRuntimeConfigUpdate `json:"runtime,omitempty"`
	Preferences *DesktopPreferencesUpdate   `json:"preferences,omitempty"`
}

// DesktopConfigValidationError describes a user-fixable Settings validation failure.
type DesktopConfigValidationError struct {
	Message     string
	Facts       []poc.Fact
	Suggestions []poc.Suggestion
}

func (e *DesktopConfigValidationError) Error() string {
	if e == nil || strings.TrimSpace(e.Message) == "" {
		return "invalid desktop config"
	}
	return e.Message
}

type desktopSettingsFile struct {
	Format      string             `json:"format"`
	Preferences DesktopPreferences `json:"preferences"`
}

func (m *Manager) UpdateDesktopConfig(update DesktopConfigUpdate) (DesktopStateSnapshot, error) {
	if update.Runtime == nil && update.Preferences == nil {
		return DesktopStateSnapshot{}, desktopConfigValidationError(
			"missing desktop config update",
			"provide runtime or preferences fields",
		)
	}

	st, err := m.loadState()
	if err != nil {
		return DesktopStateSnapshot{}, fmt.Errorf("load state: %w", err)
	}
	if st.Local == nil {
		st.Local = &pocstate.LocalConfig{}
	}
	st.Local.NormalizeDefaults()
	if update.Runtime != nil {
		if err := applyRuntimeConfigUpdate(st.Local, *update.Runtime); err != nil {
			return DesktopStateSnapshot{}, err
		}
	}

	settings, err := m.loadDesktopSettings()
	if err != nil {
		return DesktopStateSnapshot{}, fmt.Errorf("load desktop settings: %w", err)
	}
	if update.Preferences != nil {
		if err := applyPreferencesUpdate(&settings.Preferences, *update.Preferences); err != nil {
			return DesktopStateSnapshot{}, err
		}
	}

	if err := m.saveStateFile(st); err != nil {
		return DesktopStateSnapshot{}, fmt.Errorf("save state: %w", err)
	}
	if err := m.saveDesktopSettings(settings); err != nil {
		return DesktopStateSnapshot{}, fmt.Errorf("save desktop settings: %w", err)
	}
	logutil.SetLevel(settings.Preferences.LogLevel)
	m.publishDesktopConfigAndTopologyChange()

	return m.DesktopStateSnapshot()
}

func (m *Manager) applyPersistedDesktopSettings() {
	settings, err := m.loadDesktopSettings()
	if err != nil {
		return
	}
	logutil.SetLevel(settings.Preferences.LogLevel)
}

func (m *Manager) loadDesktopSettings() (desktopSettingsFile, error) {
	path, err := m.desktopSettingsPath()
	if err != nil {
		return desktopSettingsFile{}, err
	}

	data, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return defaultDesktopSettings(), nil
		}
		return desktopSettingsFile{}, err
	}

	var settings desktopSettingsFile
	if err := json.Unmarshal(data, &settings); err != nil {
		return desktopSettingsFile{}, fmt.Errorf("unmarshal desktop settings: %w", err)
	}
	if strings.TrimSpace(settings.Format) == "" {
		settings.Format = desktopSettingsFormatV0
	}
	normalizeDesktopPreferences(&settings.Preferences)
	return settings, nil
}

func (m *Manager) saveDesktopSettings(settings desktopSettingsFile) error {
	path, err := m.desktopSettingsPath()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return fmt.Errorf("mkdir desktop settings dir: %w", err)
	}

	if strings.TrimSpace(settings.Format) == "" {
		settings.Format = desktopSettingsFormatV0
	}
	normalizeDesktopPreferences(&settings.Preferences)
	data, err := json.MarshalIndent(settings, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal desktop settings: %w", err)
	}
	data = append(data, '\n')
	return atomicfile.WriteFile(path, data, 0o600)
}

func (m *Manager) desktopSettingsPath() (string, error) {
	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		return "", err
	}
	return filepath.Join(stateDir, "desktop_settings.json"), nil
}

func defaultDesktopSettings() desktopSettingsFile {
	return desktopSettingsFile{
		Format: desktopSettingsFormatV0,
		Preferences: DesktopPreferences{
			LogLevel: defaultDesktopLogLevel,
		},
	}
}

func applyRuntimeConfigUpdate(cfg *pocstate.LocalConfig, update DesktopRuntimeConfigUpdate) error {
	if cfg == nil {
		return errors.New("nil local config")
	}

	if update.MQTTBrokers != nil {
		brokers := normalizeStringList(update.MQTTBrokers)
		if len(brokers) == 0 {
			return desktopConfigValidationError("invalid MQTT brokers", "provide at least one broker endpoint")
		}
		if err := validateHostPortList("MQTT brokers", brokers, false); err != nil {
			return err
		}
		cfg.SetMQTTBrokers(brokers)
	}
	if strings.TrimSpace(update.P2PNetwork) != "" {
		value := strings.TrimSpace(update.P2PNetwork)
		if !isValidP2PNetwork(value) {
			return desktopConfigValidationError("invalid p2p_network", "use auto, udp_only, or tcp_only")
		}
		cfg.P2PNetwork = value
	}
	if strings.TrimSpace(update.P2PIPFamily) != "" {
		value := strings.TrimSpace(update.P2PIPFamily)
		if !isValidP2PIPFamily(value) {
			return desktopConfigValidationError("invalid p2p_ip_family", "use auto, v4, or v6")
		}
		cfg.P2PIPFamily = value
	}
	if strings.TrimSpace(update.DataProto) != "" {
		value := strings.TrimSpace(update.DataProto)
		if !isValidDataProto(value) {
			return desktopConfigValidationError("invalid data_proto", "use quic or kcp")
		}
		cfg.DataProto = value
	}
	if strings.TrimSpace(update.QUICCC) != "" {
		value := strings.TrimSpace(update.QUICCC)
		if !isValidQUICCC(value) {
			return desktopConfigValidationError("invalid quic_cc", "use bbr or brutal")
		}
		cfg.QUICCC = value
	}
	if update.StunServers != nil {
		stunServers := normalizeStringList(update.StunServers)
		if err := validateSTUNList(stunServers); err != nil {
			return err
		}
		cfg.StunServers = stunServers
		cfg.StunExplicit = true
	}
	if update.StunExplicit != nil {
		cfg.StunExplicit = *update.StunExplicit
	}
	if update.DisablePortMap != nil {
		cfg.DisablePortMap = *update.DisablePortMap
	}
	if update.DisableAssistedAddrs != nil {
		cfg.DisableAssistedAddrs = *update.DisableAssistedAddrs
	}

	cfg.NormalizeDefaults()
	return nil
}

func applyPreferencesUpdate(prefs *DesktopPreferences, update DesktopPreferencesUpdate) error {
	if prefs == nil {
		return errors.New("nil desktop preferences")
	}

	if update.DefaultShellTarget != nil {
		prefs.DefaultShellTarget = strings.TrimSpace(*update.DefaultShellTarget)
	}
	if update.DefaultShellSession != nil {
		prefs.DefaultShellSession = strings.TrimSpace(*update.DefaultShellSession)
	}
	if strings.TrimSpace(update.LogLevel) != "" {
		value := strings.TrimSpace(update.LogLevel)
		if !isValidLogLevel(value) {
			return desktopConfigValidationError("invalid log_level", "use trace, debug, info, warn, or error")
		}
		prefs.LogLevel = value
	}
	normalizeDesktopPreferences(prefs)
	return nil
}

func normalizeDesktopPreferences(prefs *DesktopPreferences) {
	if prefs == nil {
		return
	}
	prefs.DefaultShellTarget = strings.TrimSpace(prefs.DefaultShellTarget)
	prefs.DefaultShellSession = strings.TrimSpace(prefs.DefaultShellSession)
	prefs.LogLevel = strings.TrimSpace(prefs.LogLevel)
	if prefs.LogLevel == "" {
		prefs.LogLevel = defaultDesktopLogLevel
	}
}

func desktopConfigValidationError(message string, suggestion string) *DesktopConfigValidationError {
	return &DesktopConfigValidationError{
		Message: message,
		Facts: []poc.Fact{
			{Message: message},
		},
		Suggestions: []poc.Suggestion{
			{Message: suggestion},
		},
	}
}

func desktopRuntimeConfigFromLocal(cfg *pocstate.LocalConfig) DesktopRuntimeConfig {
	if cfg == nil {
		cfg = &pocstate.LocalConfig{}
	}
	local := *cfg
	local.NormalizeDefaults()
	return DesktopRuntimeConfig{
		MQTTBrokers:          append([]string(nil), local.MQTTBrokerEndpoints()...),
		P2PNetwork:           strings.TrimSpace(local.P2PNetwork),
		P2PIPFamily:          strings.TrimSpace(local.P2PIPFamily),
		DataProto:            strings.TrimSpace(local.DataProto),
		QUICCC:               strings.TrimSpace(local.QUICCC),
		StunServers:          append([]string(nil), local.StunServers...),
		StunExplicit:         local.StunExplicit,
		DisablePortMap:       local.DisablePortMap,
		DisableAssistedAddrs: local.DisableAssistedAddrs,
	}
}

func normalizeStringList(values []string) []string {
	out := make([]string, 0, len(values))
	seen := make(map[string]bool, len(values))
	for _, value := range values {
		value = strings.TrimSpace(value)
		if value == "" || seen[value] {
			continue
		}
		seen[value] = true
		out = append(out, value)
	}
	return out
}

func validateSTUNList(values []string) error {
	for _, value := range values {
		parsed, err := stunclient.ParseEndpoint(value)
		if err != nil {
			return desktopConfigValidationError("invalid STUN endpoint", "use host:port, udp://host:port, or tcp://host:port")
		}
		if err := validateHostPort("STUN endpoint", parsed.HostPort); err != nil {
			return err
		}
	}
	return nil
}

func validateHostPortList(field string, values []string, allowEmpty bool) error {
	if len(values) == 0 && !allowEmpty {
		return desktopConfigValidationError("invalid "+field, "provide at least one host:port endpoint")
	}
	for _, value := range values {
		if err := validateHostPort(field, value); err != nil {
			return err
		}
	}
	return nil
}

func validateHostPort(field string, value string) error {
	host, port, err := net.SplitHostPort(strings.TrimSpace(value))
	if err != nil {
		return desktopConfigValidationError("invalid "+field, "use host:port")
	}
	host = strings.Trim(strings.TrimSpace(host), "[]")
	if host == "" {
		return desktopConfigValidationError("invalid "+field, "provide a host")
	}
	portNumber, err := strconv.Atoi(port)
	if err != nil || portNumber < 1 || portNumber > 65535 {
		return desktopConfigValidationError("invalid "+field, "provide a TCP or UDP port from 1 to 65535")
	}
	return nil
}

func isValidP2PNetwork(value string) bool {
	switch value {
	case "auto", "udp_only", "tcp_only":
		return true
	default:
		return false
	}
}

func isValidP2PIPFamily(value string) bool {
	switch value {
	case "auto", "v4", "v6":
		return true
	default:
		return false
	}
}

func isValidDataProto(value string) bool {
	switch value {
	case "quic", "kcp":
		return true
	default:
		return false
	}
}

func isValidQUICCC(value string) bool {
	switch value {
	case "bbr", "brutal":
		return true
	default:
		return false
	}
}

func isValidLogLevel(value string) bool {
	switch value {
	case "trace", "debug", "info", "warn", "error":
		return true
	default:
		return false
	}
}
