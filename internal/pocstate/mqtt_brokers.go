package pocstate

import (
	"encoding/json"
	"fmt"
	"strings"
)

func mqttBrokerEndpoints(primary string, brokers []string) []string {
	out := normalizeBrokers(brokers)
	if len(out) > 0 {
		return out
	}
	if strings.TrimSpace(primary) == "" {
		return nil
	}
	return normalizeBrokers([]string{primary})
}

func primaryMQTTBroker(primary string, brokers []string) string {
	normalized := mqttBrokerEndpoints(primary, brokers)
	if len(normalized) == 0 {
		return ""
	}
	return normalized[0]
}

func decodeMQTTBrokerField(data json.RawMessage) ([]string, error) {
	trimmed := strings.TrimSpace(string(data))
	if trimmed == "" || trimmed == "null" {
		return nil, nil
	}

	var single string
	if err := json.Unmarshal(data, &single); err == nil {
		return mqttBrokerEndpoints(single, nil), nil
	}

	var many []string
	if err := json.Unmarshal(data, &many); err == nil {
		return mqttBrokerEndpoints("", many), nil
	}

	return nil, fmt.Errorf("mqtt_broker must be string or string array")
}

type localConfigJSON struct {
	PeerID      string          `json:"peer_id"`
	ProxyName   string          `json:"proxy_name"`
	SecretKey   string          `json:"secret_key"`
	MQTTBroker  json.RawMessage `json:"mqtt_broker,omitempty"`
	TopicPrefix string          `json:"topic_prefix"`

	V4Hint string `json:"v4_hint,omitempty"`
	V6Hint string `json:"v6_hint,omitempty"`

	DataProto string `json:"data_proto"`
	QUICCC    string `json:"quic_cc"`

	P2PNetwork  string `json:"p2p_network,omitempty"`
	P2PIPFamily string `json:"p2p_ip_family,omitempty"`
	P2PPort     int    `json:"p2p_port,omitempty"`

	StunServers  []string `json:"stun,omitempty"`
	StunExplicit bool     `json:"stun_explicit,omitempty"`

	DisablePortMap       bool `json:"disable_portmap,omitempty"`
	DisableAssistedAddrs bool `json:"disable_assisted_addrs,omitempty"`
}

func (c *LocalConfig) UnmarshalJSON(data []byte) error {
	var raw localConfigJSON
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	brokers, err := decodeMQTTBrokerField(raw.MQTTBroker)
	if err != nil {
		return err
	}
	*c = LocalConfig{
		PeerID:               raw.PeerID,
		ProxyName:            raw.ProxyName,
		SecretKey:            raw.SecretKey,
		TopicPrefix:          raw.TopicPrefix,
		V4Hint:               raw.V4Hint,
		V6Hint:               raw.V6Hint,
		DataProto:            raw.DataProto,
		QUICCC:               raw.QUICCC,
		P2PNetwork:           raw.P2PNetwork,
		P2PIPFamily:          raw.P2PIPFamily,
		P2PPort:              raw.P2PPort,
		StunServers:          append([]string(nil), raw.StunServers...),
		StunExplicit:         raw.StunExplicit,
		DisablePortMap:       raw.DisablePortMap,
		DisableAssistedAddrs: raw.DisableAssistedAddrs,
	}
	c.SetMQTTBrokers(brokers)
	return nil
}

func (c LocalConfig) MarshalJSON() ([]byte, error) {
	type localConfigOut struct {
		PeerID      string   `json:"peer_id"`
		ProxyName   string   `json:"proxy_name"`
		SecretKey   string   `json:"secret_key"`
		MQTTBroker  []string `json:"mqtt_broker,omitempty"`
		TopicPrefix string   `json:"topic_prefix"`

		V4Hint string `json:"v4_hint,omitempty"`
		V6Hint string `json:"v6_hint,omitempty"`

		DataProto string `json:"data_proto"`
		QUICCC    string `json:"quic_cc"`

		P2PNetwork  string `json:"p2p_network,omitempty"`
		P2PIPFamily string `json:"p2p_ip_family,omitempty"`
		P2PPort     int    `json:"p2p_port,omitempty"`

		StunServers  []string `json:"stun,omitempty"`
		StunExplicit bool     `json:"stun_explicit,omitempty"`

		DisablePortMap       bool `json:"disable_portmap,omitempty"`
		DisableAssistedAddrs bool `json:"disable_assisted_addrs,omitempty"`
	}
	return json.Marshal(localConfigOut{
		PeerID:               c.PeerID,
		ProxyName:            c.ProxyName,
		SecretKey:            c.SecretKey,
		MQTTBroker:           c.MQTTBrokerEndpoints(),
		TopicPrefix:          c.TopicPrefix,
		V4Hint:               c.V4Hint,
		V6Hint:               c.V6Hint,
		DataProto:            c.DataProto,
		QUICCC:               c.QUICCC,
		P2PNetwork:           c.P2PNetwork,
		P2PIPFamily:          c.P2PIPFamily,
		P2PPort:              c.P2PPort,
		StunServers:          append([]string(nil), c.StunServers...),
		StunExplicit:         c.StunExplicit,
		DisablePortMap:       c.DisablePortMap,
		DisableAssistedAddrs: c.DisableAssistedAddrs,
	})
}

func (c LocalConfig) MQTTBrokerEndpoints() []string {
	return append([]string(nil), mqttBrokerEndpoints(c.MQTTBroker, c.MQTTBrokers)...)
}

func (c *LocalConfig) SetMQTTBrokers(brokers []string) {
	if c == nil {
		return
	}
	c.MQTTBrokers = append([]string(nil), mqttBrokerEndpoints("", brokers)...)
	c.MQTTBroker = primaryMQTTBroker("", c.MQTTBrokers)
}

func (c LocalConfig) HasExplicitMQTTBrokerConfig() bool {
	return len(c.MQTTBrokerEndpoints()) > 0
}

type peerConfigJSON struct {
	ProxyName   string          `json:"proxy_name"`
	SecretKey   string          `json:"secret_key"`
	MQTTBroker  json.RawMessage `json:"mqtt_broker,omitempty"`
	TopicPrefix string          `json:"topic_prefix"`

	V4Hint string `json:"v4_hint,omitempty"`
	V6Hint string `json:"v6_hint,omitempty"`

	DataProto string `json:"data_proto"`
	QUICCC    string `json:"quic_cc"`

	P2PNetwork  string `json:"p2p_network,omitempty"`
	P2PIPFamily string `json:"p2p_ip_family,omitempty"`
	P2PPort     int    `json:"p2p_port,omitempty"`

	StunServers  []string `json:"stun,omitempty"`
	StunExplicit bool     `json:"stun_explicit,omitempty"`

	DisablePortMap       bool `json:"disable_portmap,omitempty"`
	DisableAssistedAddrs bool `json:"disable_assisted_addrs,omitempty"`
}

func (c *PeerConfig) UnmarshalJSON(data []byte) error {
	var raw peerConfigJSON
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	brokers, err := decodeMQTTBrokerField(raw.MQTTBroker)
	if err != nil {
		return err
	}
	*c = PeerConfig{
		ProxyName:            raw.ProxyName,
		SecretKey:            raw.SecretKey,
		TopicPrefix:          raw.TopicPrefix,
		V4Hint:               raw.V4Hint,
		V6Hint:               raw.V6Hint,
		DataProto:            raw.DataProto,
		QUICCC:               raw.QUICCC,
		P2PNetwork:           raw.P2PNetwork,
		P2PIPFamily:          raw.P2PIPFamily,
		P2PPort:              raw.P2PPort,
		StunServers:          append([]string(nil), raw.StunServers...),
		StunExplicit:         raw.StunExplicit,
		DisablePortMap:       raw.DisablePortMap,
		DisableAssistedAddrs: raw.DisableAssistedAddrs,
	}
	c.SetMQTTBrokers(brokers)
	return nil
}

func (c PeerConfig) MarshalJSON() ([]byte, error) {
	type peerConfigOut struct {
		ProxyName   string   `json:"proxy_name"`
		SecretKey   string   `json:"secret_key"`
		MQTTBroker  []string `json:"mqtt_broker,omitempty"`
		TopicPrefix string   `json:"topic_prefix"`

		V4Hint string `json:"v4_hint,omitempty"`
		V6Hint string `json:"v6_hint,omitempty"`

		DataProto string `json:"data_proto"`
		QUICCC    string `json:"quic_cc"`

		P2PNetwork  string `json:"p2p_network,omitempty"`
		P2PIPFamily string `json:"p2p_ip_family,omitempty"`
		P2PPort     int    `json:"p2p_port,omitempty"`

		StunServers  []string `json:"stun,omitempty"`
		StunExplicit bool     `json:"stun_explicit,omitempty"`

		DisablePortMap       bool `json:"disable_portmap,omitempty"`
		DisableAssistedAddrs bool `json:"disable_assisted_addrs,omitempty"`
	}
	return json.Marshal(peerConfigOut{
		ProxyName:            c.ProxyName,
		SecretKey:            c.SecretKey,
		MQTTBroker:           c.MQTTBrokerEndpoints(),
		TopicPrefix:          c.TopicPrefix,
		V4Hint:               c.V4Hint,
		V6Hint:               c.V6Hint,
		DataProto:            c.DataProto,
		QUICCC:               c.QUICCC,
		P2PNetwork:           c.P2PNetwork,
		P2PIPFamily:          c.P2PIPFamily,
		P2PPort:              c.P2PPort,
		StunServers:          append([]string(nil), c.StunServers...),
		StunExplicit:         c.StunExplicit,
		DisablePortMap:       c.DisablePortMap,
		DisableAssistedAddrs: c.DisableAssistedAddrs,
	})
}

func (c PeerConfig) MQTTBrokerEndpoints() []string {
	return append([]string(nil), mqttBrokerEndpoints(c.MQTTBroker, c.MQTTBrokers)...)
}

func (c *PeerConfig) SetMQTTBrokers(brokers []string) {
	if c == nil {
		return
	}
	c.MQTTBrokers = append([]string(nil), mqttBrokerEndpoints("", brokers)...)
	c.MQTTBroker = primaryMQTTBroker("", c.MQTTBrokers)
}
