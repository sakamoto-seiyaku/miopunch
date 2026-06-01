package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net/url"
	"os"
	"path/filepath"
	"strings"

	"github.com/miopunch/miopunch/internal/pocstate"
)

const (
	mnt02SeedFormatV0 = "miopunch.mnt02.seed.v0"

	mnt02SeedDefaultTopicPrefix = "miopunch/mnt02"
	mnt02SeedDefaultProxyName   = "mnt02"
	mnt02SeedDefaultSecretKey   = "mnt02-secret"
)

type mnt02SeedConfig struct {
	StateRoot string
	PeerCount int

	MQTTBroker  string
	TopicPrefix string

	ProxyName string
	SecretKey string

	DataProto string
	QUICCC    string

	P2PNetwork  string
	P2PIPFamily string
	P2PPortBase int
	P2PPortStep int

	StunServers []string
	DisableStun bool

	DisablePortMap bool

	OutEnv  string
	OutJSON string
}

type mnt02SeedPeer struct {
	Index int `json:"index"`

	PeerID    string `json:"peer_id"`
	StatePath string `json:"state_path"`

	P2PPort int `json:"p2p_port"`
}

type mnt02SeedOutput struct {
	Format string `json:"format"`

	PeerCount int    `json:"peer_count"`
	StateRoot string `json:"state_root"`

	MQTTBroker  string `json:"mqtt_broker"`
	TopicPrefix string `json:"topic_prefix"`

	Peers []mnt02SeedPeer `json:"peers"`

	InjectedAllowed map[string]any `json:"injected_allowed"`
	NotInjected     []string       `json:"not_injected"`
}

func mnt02SeedCmd(_ context.Context, args []string, stdout, stderr io.Writer) int {
	cfg, err := parseMNT02SeedFlags(args, stderr)
	if err != nil {
		if errors.Is(err, flag.ErrHelp) {
			return 0
		}
		fmt.Fprintln(stderr, err)
		return 2
	}

	out, err := runMNT02Seed(cfg)
	if err != nil {
		fmt.Fprintln(stderr, err)
		return 1
	}

	if cfg.OutEnv != "" {
		if err := writeMNT02SeedEnv(cfg.OutEnv, out); err != nil {
			fmt.Fprintln(stderr, err)
			return 1
		}
	}
	if cfg.OutJSON != "" {
		if err := writeMNT02SeedJSON(cfg.OutJSON, out); err != nil {
			fmt.Fprintln(stderr, err)
			return 1
		}
	}

	data, err := json.Marshal(out)
	if err != nil {
		fmt.Fprintln(stderr, err)
		return 1
	}
	fmt.Fprintln(stdout, string(data))
	return 0
}

func parseMNT02SeedFlags(args []string, stderr io.Writer) (mnt02SeedConfig, error) {
	cfg := mnt02SeedConfig{
		PeerCount: 6,

		TopicPrefix: mnt02SeedDefaultTopicPrefix,
		ProxyName:   mnt02SeedDefaultProxyName,
		SecretKey:   mnt02SeedDefaultSecretKey,

		DataProto: "quic",
		QUICCC:    "bbr",

		P2PNetwork:  "auto",
		P2PIPFamily: "auto",
		P2PPortBase: 5000,
		P2PPortStep: 1,
	}

	fs := flag.NewFlagSet("mnt02-seed", flag.ContinueOnError)
	fs.SetOutput(stderr)
	fs.StringVar(&cfg.StateRoot, "state-root", "", "state root directory (contains per-peer state.json)")
	fs.IntVar(&cfg.PeerCount, "peers", cfg.PeerCount, "number of peers to seed")
	fs.StringVar(&cfg.MQTTBroker, "mqtt-broker", "", "self-hosted MQTT broker endpoint (required)")
	fs.StringVar(&cfg.TopicPrefix, "topic-prefix", cfg.TopicPrefix, "MQTT topic prefix")
	fs.StringVar(&cfg.ProxyName, "proxy", cfg.ProxyName, "proxy/session name prefix")
	fs.StringVar(&cfg.SecretKey, "secret", cfg.SecretKey, "shared secret prefix (used to derive per-peer secrets)")
	fs.StringVar(&cfg.DataProto, "data-proto", cfg.DataProto, "data plane protocol")
	fs.StringVar(&cfg.QUICCC, "quic-cc", cfg.QUICCC, "QUIC congestion control")
	fs.StringVar(&cfg.P2PNetwork, "p2p-network", cfg.P2PNetwork, "p2p network policy")
	fs.StringVar(&cfg.P2PIPFamily, "p2p-ip-family", cfg.P2PIPFamily, "peer p2p IP family")
	fs.IntVar(&cfg.P2PPortBase, "p2p-port-base", cfg.P2PPortBase, "per-peer pinned p2p port base")
	fs.IntVar(&cfg.P2PPortStep, "p2p-port-step", cfg.P2PPortStep, "per-peer pinned p2p port step")
	stunRaw := fs.String("stun", "", "comma-separated STUN endpoints (optional)")
	fs.BoolVar(&cfg.DisableStun, "disable-stun", false, "configure explicit empty STUN list")
	fs.BoolVar(&cfg.DisablePortMap, "disable-portmap", false, "disable portmap helpers")
	fs.StringVar(&cfg.OutEnv, "out-env", "", "write shell env output")
	fs.StringVar(&cfg.OutJSON, "out-json", "", "write JSON output")
	if err := fs.Parse(args); err != nil {
		return mnt02SeedConfig{}, err
	}

	cfg.StunServers = splitComma(*stunRaw)
	return cfg, validateMNT02SeedConfig(cfg)
}

func validateMNT02SeedConfig(cfg mnt02SeedConfig) error {
	if strings.TrimSpace(cfg.StateRoot) == "" {
		return errors.New("missing --state-root")
	}
	if cfg.PeerCount < 2 || cfg.PeerCount > 32 {
		return fmt.Errorf("invalid --peers: %d (want 2..32)", cfg.PeerCount)
	}
	if strings.TrimSpace(cfg.MQTTBroker) == "" {
		return errors.New("missing --mqtt-broker")
	}
	if isDisallowedMNT02MQTTBroker(cfg.MQTTBroker) {
		return errors.New("public MQTT broker is not allowed for mnt02-seed; set --mqtt-broker to the self-hosted broker")
	}
	if strings.TrimSpace(cfg.ProxyName) == "" {
		return errors.New("missing --proxy")
	}
	if strings.TrimSpace(cfg.SecretKey) == "" {
		return errors.New("missing --secret")
	}
	if cfg.P2PPortBase < 0 || cfg.P2PPortBase > 65535 {
		return fmt.Errorf("invalid --p2p-port-base: %d", cfg.P2PPortBase)
	}
	if cfg.P2PPortStep <= 0 || cfg.P2PPortStep > 65535 {
		return fmt.Errorf("invalid --p2p-port-step: %d", cfg.P2PPortStep)
	}
	if cfg.DisableStun && len(cfg.StunServers) > 0 {
		return errors.New("--disable-stun conflicts with --stun")
	}
	return nil
}

func isDisallowedMNT02MQTTBroker(raw string) bool {
	ep := normalizeHostPort(raw)
	return ep == "mqtt.eclipseprojects.io:1883"
}

func normalizeHostPort(raw string) string {
	v := strings.TrimSpace(raw)
	if v == "" {
		return ""
	}
	if !strings.Contains(v, "://") {
		return v
	}
	u, err := url.Parse(v)
	if err != nil {
		return v
	}
	return strings.TrimSpace(u.Host)
}

func runMNT02Seed(cfg mnt02SeedConfig) (mnt02SeedOutput, error) {
	stateRoot := strings.TrimSpace(cfg.StateRoot)

	stunServers := append([]string(nil), cfg.StunServers...)
	if cfg.DisableStun {
		stunServers = nil
	}

	peers := make([]mnt02SeedPeer, 0, cfg.PeerCount)
	p2pPorts := make([]int, 0, cfg.PeerCount)

	for i := 1; i <= cfg.PeerCount; i++ {
		statePath := filepath.Join(stateRoot, fmt.Sprintf("p%d", i), "state.json")
		stateDir, err := pocstate.StateDir(statePath)
		if err != nil {
			return mnt02SeedOutput{}, fmt.Errorf("peer %d state dir: %w", i, err)
		}

		id, err := pocstate.EnsureIdentity(stateDir)
		if err != nil {
			return mnt02SeedOutput{}, fmt.Errorf("peer %d ensure identity: %w", i, err)
		}

		p2pPort := cfg.P2PPortBase + (i-1)*cfg.P2PPortStep
		if p2pPort < 0 || p2pPort > 65535 {
			return mnt02SeedOutput{}, fmt.Errorf("peer %d p2p_port out of range: %d", i, p2pPort)
		}
		p2pPorts = append(p2pPorts, p2pPort)

		local := pocstate.LocalConfig{
			PeerID:      id.PeerID,
			ProxyName:   fmt.Sprintf("%s-p%d", cfg.ProxyName, i),
			SecretKey:   fmt.Sprintf("%s-p%d", cfg.SecretKey, i),
			MQTTBroker:  cfg.MQTTBroker,
			TopicPrefix: cfg.TopicPrefix,
			DataProto:   cfg.DataProto,
			QUICCC:      cfg.QUICCC,
			P2PNetwork:  cfg.P2PNetwork,
			P2PIPFamily: cfg.P2PIPFamily,
			P2PPort:     p2pPort,

			StunServers:  stunServers,
			StunExplicit: true, // never fall back to builtin/public STUN in required gates

			DisablePortMap: cfg.DisablePortMap,
		}

		st := pocstate.State{
			Format: pocstate.FormatV0,
			Local:  &local,
			Peers:  map[string]pocstate.PeerConfig{},
		}
		if err := pocstate.Save(statePath, st); err != nil {
			return mnt02SeedOutput{}, fmt.Errorf("peer %d save state: %w", i, err)
		}

		peers = append(peers, mnt02SeedPeer{
			Index:     i,
			PeerID:    id.PeerID,
			StatePath: statePath,
			P2PPort:   p2pPort,
		})
	}

	return mnt02SeedOutput{
		Format:      mnt02SeedFormatV0,
		PeerCount:   cfg.PeerCount,
		StateRoot:   stateRoot,
		MQTTBroker:  cfg.MQTTBroker,
		TopicPrefix: cfg.TopicPrefix,
		Peers:       peers,
		InjectedAllowed: map[string]any{
			"identity":        true,
			"peer_config":     true,
			"mqtt_broker":     cfg.MQTTBroker,
			"topic_prefix":    cfg.TopicPrefix,
			"stun":            stunServers,
			"stun_explicit":   true,
			"disable_portmap": cfg.DisablePortMap,
			"test_ports":      p2pPorts,
			"p2p_network":     cfg.P2PNetwork,
			"p2p_ip_family":   cfg.P2PIPFamily,
		},
		NotInjected: []string{
			"net.json",
			"governance/head_snapshot.json",
			"decls/decls.json",
			"peers (remote peer list)",
			"membership approvals",
			"task outcomes",
		},
	}, nil
}

func writeMNT02SeedEnv(path string, out mnt02SeedOutput) error {
	var b strings.Builder
	fmt.Fprintf(&b, "peer_count=%d\n", out.PeerCount)
	fmt.Fprintf(&b, "state_root=%s\n", out.StateRoot)
	fmt.Fprintf(&b, "mqtt_broker=%s\n", out.MQTTBroker)
	fmt.Fprintf(&b, "topic_prefix=%s\n", out.TopicPrefix)
	for _, p := range out.Peers {
		fmt.Fprintf(&b, "peer_%d_id=%s\n", p.Index, p.PeerID)
		fmt.Fprintf(&b, "peer_%d_state=%s\n", p.Index, p.StatePath)
		fmt.Fprintf(&b, "peer_%d_p2p_port=%d\n", p.Index, p.P2PPort)
	}
	return writeMNT02File(path, []byte(b.String()))
}

func writeMNT02SeedJSON(path string, out mnt02SeedOutput) error {
	data, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal seed json: %w", err)
	}
	data = append(data, '\n')
	return writeMNT02File(path, data)
}

func writeMNT02File(path string, data []byte) error {
	if strings.TrimSpace(path) == "" {
		return nil
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("mkdir output dir: %w", err)
	}
	if err := os.WriteFile(path, data, 0o644); err != nil {
		return fmt.Errorf("write %s: %w", path, err)
	}
	return nil
}
