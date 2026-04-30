package pocstate

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const (
	FormatV0 = "miopunch.state.poc.v0"

	defaultTopicPrefix = "miopunch/poc"
	defaultMQTTBroker  = "mqtt.eclipseprojects.io:1883"
	defaultDataProto   = "quic"
	defaultQUICCC      = "bbr"
	defaultP2PNetwork  = "auto"
)

type State struct {
	Format string `json:"format"`

	Local *LocalConfig          `json:"local,omitempty"`
	Peers map[string]PeerConfig `json:"peers,omitempty"` // peer_id -> config
}

type LocalConfig struct {
	PeerID      string `json:"peer_id"`
	ProxyName   string `json:"proxy_name"`
	SecretKey   string `json:"secret_key"`
	MQTTBroker  string `json:"mqtt_broker"`
	TopicPrefix string `json:"topic_prefix"`

	V4Hint string `json:"v4_hint,omitempty"`
	V6Hint string `json:"v6_hint,omitempty"`

	DataProto string `json:"data_proto"` // quic | kcp
	QUICCC    string `json:"quic_cc"`    // bbr | brutal (only when data_proto=quic)

	P2PNetwork  string `json:"p2p_network,omitempty"`   // auto | udp_only | tcp_only
	P2PIPFamily string `json:"p2p_ip_family,omitempty"` // auto | v4 | v6
	P2PPort     int    `json:"p2p_port,omitempty"`

	StunServers  []string `json:"stun,omitempty"`
	StunExplicit bool     `json:"stun_explicit,omitempty"`

	DisablePortMap       bool `json:"disable_portmap,omitempty"`
	DisableAssistedAddrs bool `json:"disable_assisted_addrs,omitempty"`
}

type PeerConfig struct {
	ProxyName   string `json:"proxy_name"`
	SecretKey   string `json:"secret_key"`
	MQTTBroker  string `json:"mqtt_broker"`
	TopicPrefix string `json:"topic_prefix"`

	V4Hint string `json:"v4_hint,omitempty"`
	V6Hint string `json:"v6_hint,omitempty"`

	DataProto string `json:"data_proto"` // quic | kcp
	QUICCC    string `json:"quic_cc"`    // bbr | brutal (only when data_proto=quic)

	P2PNetwork  string `json:"p2p_network,omitempty"`   // auto | udp_only | tcp_only
	P2PIPFamily string `json:"p2p_ip_family,omitempty"` // auto | v4 | v6
	P2PPort     int    `json:"p2p_port,omitempty"`

	StunServers  []string `json:"stun,omitempty"`
	StunExplicit bool     `json:"stun_explicit,omitempty"`

	DisablePortMap       bool `json:"disable_portmap,omitempty"`
	DisableAssistedAddrs bool `json:"disable_assisted_addrs,omitempty"`
}

func Load(path string) (State, error) {
	if strings.TrimSpace(path) == "" {
		return State{}, errors.New("empty state path")
	}

	data, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return State{Format: FormatV0, Peers: map[string]PeerConfig{}}, nil
		}
		return State{}, err
	}

	var st State
	if err := json.Unmarshal(data, &st); err != nil {
		return State{}, fmt.Errorf("unmarshal state: %w", err)
	}
	if strings.TrimSpace(st.Format) == "" {
		st.Format = FormatV0
	}
	if st.Peers == nil {
		st.Peers = map[string]PeerConfig{}
	}
	return st, nil
}

func Save(path string, st State) error {
	if strings.TrimSpace(path) == "" {
		return errors.New("empty state path")
	}

	st = st.Clone()
	if strings.TrimSpace(st.Format) == "" {
		st.Format = FormatV0
	}
	if st.Peers == nil {
		st.Peers = map[string]PeerConfig{}
	}

	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("mkdir state dir: %w", err)
	}

	data, err := json.MarshalIndent(st, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal state: %w", err)
	}
	data = append(data, '\n')

	if err := writeFileAtomic(path, data, 0o600); err != nil {
		return fmt.Errorf("write state: %w", err)
	}
	return nil
}

func (s State) Clone() State {
	out := s
	if s.Local != nil {
		localCopy := *s.Local
		out.Local = &localCopy
	}
	if s.Peers != nil {
		out.Peers = make(map[string]PeerConfig, len(s.Peers))
		for k, v := range s.Peers {
			out.Peers[k] = v
		}
	}
	return out
}

func (s *State) UpsertPeer(peerID string, cfg PeerConfig) {
	if s == nil {
		return
	}
	peerID = strings.TrimSpace(peerID)
	if peerID == "" {
		return
	}
	if s.Peers == nil {
		s.Peers = map[string]PeerConfig{}
	}
	cfg.NormalizeDefaults()
	s.Peers[peerID] = cfg
}

func (s *State) EnsureLocalDefaults() {
	if s == nil || s.Local == nil {
		return
	}
	s.Local.NormalizeDefaults()
}

func (c LocalConfig) ToPeer() PeerConfig {
	return PeerConfig{
		ProxyName:   c.ProxyName,
		SecretKey:   c.SecretKey,
		MQTTBroker:  c.MQTTBroker,
		TopicPrefix: c.TopicPrefix,
		V4Hint:      NormalizeV4Hint(c.V4Hint),
		V6Hint:      NormalizeV6Hint(c.V6Hint),
		DataProto:   c.DataProto,
		QUICCC:      c.QUICCC,
		P2PNetwork:  c.P2PNetwork,
		P2PIPFamily: c.P2PIPFamily,
		P2PPort:     c.P2PPort,

		StunServers:  append([]string(nil), c.StunServers...),
		StunExplicit: c.StunExplicit,

		DisablePortMap:       c.DisablePortMap,
		DisableAssistedAddrs: c.DisableAssistedAddrs,
	}
}

func (c *LocalConfig) NormalizeDefaults() {
	if c == nil {
		return
	}
	if strings.TrimSpace(c.MQTTBroker) == "" {
		c.MQTTBroker = defaultMQTTBroker
	}
	if strings.TrimSpace(c.TopicPrefix) == "" {
		c.TopicPrefix = defaultTopicPrefix
	}
	if strings.TrimSpace(c.DataProto) == "" {
		c.DataProto = defaultDataProto
	}
	if strings.TrimSpace(c.QUICCC) == "" {
		c.QUICCC = defaultQUICCC
	}
	if strings.TrimSpace(c.P2PNetwork) == "" {
		c.P2PNetwork = defaultP2PNetwork
	}
	c.V4Hint = NormalizeV4Hint(c.V4Hint)
	c.V6Hint = NormalizeV6Hint(c.V6Hint)
	c.StunServers = normalizeBrokers(c.StunServers)
}

func (c *PeerConfig) NormalizeDefaults() {
	if c == nil {
		return
	}
	if strings.TrimSpace(c.MQTTBroker) == "" {
		c.MQTTBroker = defaultMQTTBroker
	}
	if strings.TrimSpace(c.TopicPrefix) == "" {
		c.TopicPrefix = defaultTopicPrefix
	}
	if strings.TrimSpace(c.DataProto) == "" {
		c.DataProto = defaultDataProto
	}
	if strings.TrimSpace(c.QUICCC) == "" {
		c.QUICCC = defaultQUICCC
	}
	if strings.TrimSpace(c.P2PNetwork) == "" {
		c.P2PNetwork = defaultP2PNetwork
	}
	c.V4Hint = NormalizeV4Hint(c.V4Hint)
	c.V6Hint = NormalizeV6Hint(c.V6Hint)
	c.StunServers = normalizeBrokers(c.StunServers)
}
