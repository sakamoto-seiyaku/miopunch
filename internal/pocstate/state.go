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

	DataProto string `json:"data_proto"` // quic | kcp
	QUICCC    string `json:"quic_cc"`    // bbr | brutal (only when data_proto=quic)
}

type PeerConfig struct {
	ProxyName   string `json:"proxy_name"`
	SecretKey   string `json:"secret_key"`
	MQTTBroker  string `json:"mqtt_broker"`
	TopicPrefix string `json:"topic_prefix"`

	DataProto string `json:"data_proto"` // quic | kcp
	QUICCC    string `json:"quic_cc"`    // bbr | brutal (only when data_proto=quic)
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
		DataProto:   c.DataProto,
		QUICCC:      c.QUICCC,
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
}
