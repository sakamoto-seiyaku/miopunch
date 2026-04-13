// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

package main

import (
	"fmt"
	"os"
	"strings"
	"time"

	"gopkg.in/yaml.v3"
)

type yamlDuration struct {
	time.Duration
}

func (d *yamlDuration) UnmarshalYAML(node *yaml.Node) error {
	if node == nil {
		return nil
	}
	if node.Kind != yaml.ScalarNode {
		return fmt.Errorf("invalid duration: expected scalar")
	}
	value := strings.TrimSpace(node.Value)
	if value == "" {
		d.Duration = 0
		return nil
	}
	parsed, err := time.ParseDuration(value)
	if err != nil {
		return fmt.Errorf("invalid duration %q: %w", value, err)
	}
	d.Duration = parsed
	return nil
}

type peerYAMLConfig struct {
	Signaling *string `yaml:"signaling"`

	Coord        *string `yaml:"coord"`
	ControlProto *string `yaml:"control_proto"`

	Proxy      *string  `yaml:"proxy"`
	Secret     *string  `yaml:"secret"`
	User       *string  `yaml:"user"`
	AllowUsers []string `yaml:"allow_users"`

	DataProto *string `yaml:"data_proto"`
	QuicCC    *string `yaml:"quic_cc"`
	Payload   *string `yaml:"payload"`

	P2PPort *int `yaml:"p2p_port"`

	Stun []string `yaml:"stun"`

	StunTimeout           *yamlDuration `yaml:"stun_timeout"`
	GatherTimeout         *yamlDuration `yaml:"gather_timeout"`
	AttemptV6Timeout      *yamlDuration `yaml:"attempt_v6_timeout"`
	AttemptPortmapTimeout *yamlDuration `yaml:"attempt_portmap_timeout"`
	HelloTimeout          *yamlDuration `yaml:"hello_timeout"`
	ExchangeTimeout       *yamlDuration `yaml:"exchange_timeout"`
	OverallTimeout        *yamlDuration `yaml:"overall_timeout"`

	DisablePortmap  *bool `yaml:"disable_portmap"`
	DisableAssisted *bool `yaml:"disable_assisted"`
	Once            *bool `yaml:"once"`

	MQTTBroker      *string `yaml:"mqtt_broker"`
	MQTTTopicPrefix *string `yaml:"mqtt_topic_prefix"`
	MQTTUser        *string `yaml:"mqtt_user"`
	MQTTPass        *string `yaml:"mqtt_pass"`
}

func loadPeerYAMLConfig(path string) (*peerYAMLConfig, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var cfg peerYAMLConfig
	if err := yaml.Unmarshal(raw, &cfg); err != nil {
		return nil, err
	}
	return &cfg, nil
}
