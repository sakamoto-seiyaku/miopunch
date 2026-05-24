package pocstate

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"strings"
)

const (
	JoinCodePrefixV0 = "miopunch.join.v0:"
	JoinCodeURLHost  = "join"
)

type JoinCode struct {
	PeerID string `json:"peer_id"`

	ProxyName   string `json:"proxy_name"`
	SecretKey   string `json:"secret_key"`
	MQTTBroker  string `json:"mqtt_broker"`
	TopicPrefix string `json:"topic_prefix"`

	DataProto string `json:"data_proto"`
	QUICCC    string `json:"quic_cc"`
}

func (c JoinCode) Validate() error {
	if strings.TrimSpace(c.PeerID) == "" {
		return errors.New("empty peer_id")
	}
	if strings.TrimSpace(c.ProxyName) == "" {
		return errors.New("empty proxy_name")
	}
	if strings.TrimSpace(c.SecretKey) == "" {
		return errors.New("empty secret_key")
	}
	return nil
}

func (c *JoinCode) NormalizeDefaults() {
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

func EncodeJoinCodeV0(c JoinCode) (string, error) {
	c.NormalizeDefaults()
	if err := c.Validate(); err != nil {
		return "", err
	}

	data, err := json.Marshal(c)
	if err != nil {
		return "", fmt.Errorf("marshal join code: %w", err)
	}
	b64 := base64.RawURLEncoding.EncodeToString(data)
	return JoinCodePrefixV0 + b64, nil
}

func DecodeJoinCodeV0(codeOrURL string) (JoinCode, error) {
	raw := strings.TrimSpace(codeOrURL)
	if raw == "" {
		return JoinCode{}, errors.New("empty join code")
	}

	if strings.HasPrefix(raw, JoinCodePrefixV0) {
		return decodeJoinCodePayload(strings.TrimPrefix(raw, JoinCodePrefixV0))
	}

	// Minimal URL form: miopunch://join/<payload>
	if strings.HasPrefix(raw, "miopunch://") {
		u, err := url.Parse(raw)
		if err != nil {
			return JoinCode{}, fmt.Errorf("invalid join url: %w", err)
		}
		if u.Scheme != "miopunch" {
			return JoinCode{}, errors.New("invalid join url scheme")
		}
		if u.Host != JoinCodeURLHost {
			return JoinCode{}, errors.New("invalid join url host")
		}
		payload := strings.Trim(u.Path, "/")
		if payload == "" {
			payload = u.Query().Get("code")
		}
		if payload == "" {
			return JoinCode{}, errors.New("missing join url payload")
		}
		return decodeJoinCodePayload(payload)
	}

	// Bare payload fallback (copy/paste convenience).
	return decodeJoinCodePayload(raw)
}

func decodeJoinCodePayload(b64 string) (JoinCode, error) {
	data, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(b64))
	if err != nil {
		return JoinCode{}, fmt.Errorf("decode join code base64: %w", err)
	}

	var c JoinCode
	if err := json.Unmarshal(data, &c); err != nil {
		return JoinCode{}, fmt.Errorf("unmarshal join code: %w", err)
	}
	c.NormalizeDefaults()
	if err := c.Validate(); err != nil {
		return JoinCode{}, err
	}
	return c, nil
}

func (c JoinCode) ToPeerConfig() PeerConfig {
	return PeerConfig{
		ProxyName:   c.ProxyName,
		SecretKey:   c.SecretKey,
		MQTTBroker:  c.MQTTBroker,
		TopicPrefix: c.TopicPrefix,
		DataProto:   c.DataProto,
		QUICCC:      c.QUICCC,
	}
}
