package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/pocstate"
)

type mnt01SeedConfig struct {
	StateA string
	StateB string

	MQTTBroker  string
	TopicPrefix string
	ProxyName   string
	SecretKey   string

	DataProto string
	QUICCC    string

	P2PNetwork   string
	P2PIPFamilyA string
	P2PIPFamilyB string
	P2PPortA     int
	P2PPortB     int
	DialPortA    int
	DialPortB    int

	StunServers []string
	DisableStun bool

	DisablePortMapA bool
	DisablePortMapB bool

	OutEnv  string
	OutJSON string
}

type mnt01SeedOutput struct {
	Format string `json:"format"`

	PeerAID string `json:"peer_a_id"`
	PeerBID string `json:"peer_b_id"`
	NetID   string `json:"net_id"`

	StateA string `json:"state_a"`
	StateB string `json:"state_b"`

	AuthBootstrap   map[string]any `json:"auth_bootstrap"`
	InjectedAllowed map[string]any `json:"injected_allowed"`
	NotInjected     []string       `json:"not_injected"`
}

func mnt01SeedCmd(_ context.Context, args []string, stdout, stderr io.Writer) int {
	cfg, err := parseMNT01SeedFlags(args, stderr)
	if err != nil {
		if errors.Is(err, flag.ErrHelp) {
			return 0
		}
		fmt.Fprintln(stderr, err)
		return 2
	}

	out, err := runMNT01Seed(cfg)
	if err != nil {
		fmt.Fprintln(stderr, err)
		return 1
	}

	if cfg.OutEnv != "" {
		if err := writeMNT01SeedEnv(cfg.OutEnv, out); err != nil {
			fmt.Fprintln(stderr, err)
			return 1
		}
	}
	if cfg.OutJSON != "" {
		if err := writeMNT01SeedJSON(cfg.OutJSON, out); err != nil {
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

func parseMNT01SeedFlags(args []string, stderr io.Writer) (mnt01SeedConfig, error) {
	cfg := mnt01SeedConfig{
		TopicPrefix:  "miopunch/mnt01",
		ProxyName:    "mnt01",
		SecretKey:    "mnt01-secret",
		DataProto:    "quic",
		QUICCC:       "bbr",
		P2PNetwork:   "auto",
		P2PIPFamilyA: "auto",
		P2PIPFamilyB: "auto",
		P2PPortA:     5000,
		P2PPortB:     5000,
		DialPortA:    5001,
		DialPortB:    5001,
		StunServers:  nil,
	}

	fs := flag.NewFlagSet("mnt01-seed", flag.ContinueOnError)
	fs.SetOutput(stderr)
	fs.StringVar(&cfg.StateA, "state-a", "", "state path for peer A")
	fs.StringVar(&cfg.StateB, "state-b", "", "state path for peer B")
	fs.StringVar(&cfg.MQTTBroker, "mqtt-broker", "", "self-hosted MQTT broker endpoint")
	fs.StringVar(&cfg.TopicPrefix, "topic-prefix", cfg.TopicPrefix, "MQTT topic prefix")
	fs.StringVar(&cfg.ProxyName, "proxy", cfg.ProxyName, "proxy/session name")
	fs.StringVar(&cfg.SecretKey, "secret", cfg.SecretKey, "shared session secret")
	fs.StringVar(&cfg.DataProto, "data-proto", cfg.DataProto, "data plane protocol")
	fs.StringVar(&cfg.QUICCC, "quic-cc", cfg.QUICCC, "QUIC congestion control")
	fs.StringVar(&cfg.P2PNetwork, "p2p-network", cfg.P2PNetwork, "p2p network policy")
	fs.StringVar(&cfg.P2PIPFamilyA, "p2p-ip-family-a", cfg.P2PIPFamilyA, "peer A p2p IP family")
	fs.StringVar(&cfg.P2PIPFamilyB, "p2p-ip-family-b", cfg.P2PIPFamilyB, "peer B p2p IP family")
	fs.IntVar(&cfg.P2PPortA, "p2p-port-a", cfg.P2PPortA, "peer A pinned p2p port")
	fs.IntVar(&cfg.P2PPortB, "p2p-port-b", cfg.P2PPortB, "peer B pinned p2p port")
	fs.IntVar(&cfg.DialPortA, "dial-port-a", cfg.DialPortA, "peer A pinned task dial port")
	fs.IntVar(&cfg.DialPortB, "dial-port-b", cfg.DialPortB, "peer B pinned task dial port")
	stunRaw := fs.String("stun", "", "comma-separated STUN endpoints")
	fs.BoolVar(&cfg.DisableStun, "disable-stun", false, "configure explicit empty STUN list")
	fs.BoolVar(&cfg.DisablePortMapA, "disable-portmap-a", false, "disable peer A portmap helpers")
	fs.BoolVar(&cfg.DisablePortMapB, "disable-portmap-b", false, "disable peer B portmap helpers")
	fs.StringVar(&cfg.OutEnv, "out-env", "", "write shell env output")
	fs.StringVar(&cfg.OutJSON, "out-json", "", "write JSON output")
	if err := fs.Parse(args); err != nil {
		return mnt01SeedConfig{}, err
	}
	cfg.StunServers = splitComma(*stunRaw)
	return cfg, validateMNT01SeedConfig(cfg)
}

func validateMNT01SeedConfig(cfg mnt01SeedConfig) error {
	if strings.TrimSpace(cfg.StateA) == "" {
		return errors.New("missing --state-a")
	}
	if strings.TrimSpace(cfg.StateB) == "" {
		return errors.New("missing --state-b")
	}
	if strings.TrimSpace(cfg.MQTTBroker) == "" {
		return errors.New("missing --mqtt-broker")
	}
	if strings.TrimSpace(cfg.ProxyName) == "" {
		return errors.New("missing --proxy")
	}
	if strings.TrimSpace(cfg.SecretKey) == "" {
		return errors.New("missing --secret")
	}
	if cfg.P2PPortA < 0 || cfg.P2PPortA > 65535 {
		return fmt.Errorf("invalid --p2p-port-a: %d", cfg.P2PPortA)
	}
	if cfg.P2PPortB < 0 || cfg.P2PPortB > 65535 {
		return fmt.Errorf("invalid --p2p-port-b: %d", cfg.P2PPortB)
	}
	if cfg.DialPortA < 0 || cfg.DialPortA > 65535 {
		return fmt.Errorf("invalid --dial-port-a: %d", cfg.DialPortA)
	}
	if cfg.DialPortB < 0 || cfg.DialPortB > 65535 {
		return fmt.Errorf("invalid --dial-port-b: %d", cfg.DialPortB)
	}
	return nil
}

func runMNT01Seed(cfg mnt01SeedConfig) (mnt01SeedOutput, error) {
	stateDirA, err := pocstate.StateDir(cfg.StateA)
	if err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("state A dir: %w", err)
	}
	stateDirB, err := pocstate.StateDir(cfg.StateB)
	if err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("state B dir: %w", err)
	}

	identityA, err := pocstate.EnsureIdentity(stateDirA)
	if err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("ensure identity A: %w", err)
	}
	identityB, err := pocstate.EnsureIdentity(stateDirB)
	if err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("ensure identity B: %w", err)
	}

	netA, err := pocstate.EnsureNet(stateDirA, []string{cfg.MQTTBroker})
	if err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("ensure net A: %w", err)
	}
	if err := pocstate.SaveNet(stateDirB, netA); err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("save net B: %w", err)
	}

	head, err := pocstate.EnsureGovernanceHeadSnapshot(stateDirA, netA.NetID, identityA)
	if err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("ensure governance head A: %w", err)
	}
	if err := pocstate.SaveGovernanceHeadSnapshot(stateDirB, head); err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("save governance head B: %w", err)
	}

	approveB, err := pocstate.NewApproveMemberDeclV0(time.Now(), identityA, pocstate.ApproveMemberBodyV0{
		MemberPeerID:  identityB.PeerID,
		MemberName:    "mnt01-peer-b",
		Ed25519PubB64: identityB.Ed25519PubB64(),
		X25519PubB64:  identityB.X25519PubB64(),
		PlatformHint:  "mnt01-fixture",
	})
	if err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("approve peer B: %w", err)
	}
	if err := addMNT01Decl(stateDirA, approveB); err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("save approval A: %w", err)
	}
	if err := addMNT01Decl(stateDirB, approveB); err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("save approval B: %w", err)
	}

	localA := mnt01LocalConfig(cfg, identityA.PeerID, cfg.ProxyName+"-a", cfg.SecretKey+"-a", cfg.P2PPortA, cfg.P2PIPFamilyA, cfg.DisablePortMapA)
	localB := mnt01LocalConfig(cfg, identityB.PeerID, cfg.ProxyName+"-b", cfg.SecretKey+"-b", cfg.P2PPortB, cfg.P2PIPFamilyB, cfg.DisablePortMapB)
	peerFromA := mnt01PeerConfig(localB, cfg.DialPortA, cfg.P2PIPFamilyA, cfg.DisablePortMapA)
	peerFromB := mnt01PeerConfig(localA, cfg.DialPortB, cfg.P2PIPFamilyB, cfg.DisablePortMapB)
	stateA := pocstate.State{
		Format: pocstate.FormatV0,
		Local:  &localA,
		Peers: map[string]pocstate.PeerConfig{
			identityB.PeerID: peerFromA,
		},
	}
	stateB := pocstate.State{
		Format: pocstate.FormatV0,
		Local:  &localB,
		Peers: map[string]pocstate.PeerConfig{
			identityA.PeerID: peerFromB,
		},
	}
	if err := pocstate.Save(cfg.StateA, stateA); err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("save state A: %w", err)
	}
	if err := pocstate.Save(cfg.StateB, stateB); err != nil {
		return mnt01SeedOutput{}, fmt.Errorf("save state B: %w", err)
	}

	return mnt01SeedOutput{
		Format:  "miopunch.mnt01.seed.v0",
		PeerAID: identityA.PeerID,
		PeerBID: identityB.PeerID,
		NetID:   netA.NetID,
		StateA:  cfg.StateA,
		StateB:  cfg.StateB,
		AuthBootstrap: map[string]any{
			"purpose":                  "hello_auth_only",
			"governance_head_snapshot": true,
			"approve_member_decl":      true,
		},
		InjectedAllowed: map[string]any{
			"identity":        true,
			"peer_config":     true,
			"auth_bootstrap":  true,
			"mqtt_broker":     cfg.MQTTBroker,
			"stun":            cfg.StunServers,
			"test_ports":      []int{cfg.P2PPortA, cfg.P2PPortB, cfg.DialPortA, cfg.DialPortB},
			"p2p_network":     cfg.P2PNetwork,
			"p2p_ip_family_a": cfg.P2PIPFamilyA,
			"p2p_ip_family_b": cfg.P2PIPFamilyB,
			"network_profile": true,
		},
		NotInjected: []string{
			"nat_result",
			"candidate_path",
			"selected_path",
			"neighbor_state",
			"success_cache",
			"payload_result",
		},
	}, nil
}

func mnt01LocalConfig(cfg mnt01SeedConfig, peerID string, proxyName string, secretKey string, p2pPort int, p2pIPFamily string, disablePortMap bool) pocstate.LocalConfig {
	stunExplicit := cfg.DisableStun || len(cfg.StunServers) > 0
	stunServers := append([]string(nil), cfg.StunServers...)
	if cfg.DisableStun {
		stunServers = nil
	}
	return pocstate.LocalConfig{
		PeerID:         peerID,
		ProxyName:      proxyName,
		SecretKey:      secretKey,
		MQTTBroker:     cfg.MQTTBroker,
		TopicPrefix:    cfg.TopicPrefix,
		DataProto:      cfg.DataProto,
		QUICCC:         cfg.QUICCC,
		P2PNetwork:     cfg.P2PNetwork,
		P2PIPFamily:    p2pIPFamily,
		P2PPort:        p2pPort,
		StunServers:    stunServers,
		StunExplicit:   stunExplicit,
		DisablePortMap: disablePortMap,
	}
}

func mnt01PeerConfig(target pocstate.LocalConfig, dialPort int, p2pIPFamily string, disablePortMap bool) pocstate.PeerConfig {
	peer := target.ToPeer()
	peer.P2PIPFamily = p2pIPFamily
	peer.P2PPort = dialPort
	peer.DisablePortMap = disablePortMap
	return peer
}

func addMNT01Decl(stateDir string, decl pocstate.DeclV0) error {
	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		return err
	}
	_, err := pocstate.UpdateDecls(stateDir, func(file *pocstate.DeclsFileV0) error {
		file.Decls = pocstate.AddDeclSetUnionV0(file.Decls, decl)
		return nil
	})
	return err
}

func writeMNT01SeedEnv(path string, out mnt01SeedOutput) error {
	content := fmt.Sprintf(
		"peer_a_id=%s\npeer_b_id=%s\nnet_id=%s\nstate_a=%s\nstate_b=%s\n",
		out.PeerAID,
		out.PeerBID,
		out.NetID,
		out.StateA,
		out.StateB,
	)
	return writeMNT01File(path, []byte(content))
}

func writeMNT01SeedJSON(path string, out mnt01SeedOutput) error {
	data, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal seed json: %w", err)
	}
	data = append(data, '\n')
	return writeMNT01File(path, data)
}

func writeMNT01File(path string, data []byte) error {
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
