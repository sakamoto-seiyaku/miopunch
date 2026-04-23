package desktopbridge

import "github.com/miopunch/miopunch/internal/poc"

type Endpoint string

const (
	EndpointNone     Endpoint = "none"
	EndpointSystem   Endpoint = "system"
	EndpointUser     Endpoint = "user"
	EndpointOverride Endpoint = "override"
)

type BridgeError struct {
	Stage       string           `json:"stage"`
	ReasonCode  poc.ReasonCode   `json:"reason_code"`
	ExitCode    poc.ExitCode     `json:"exit_code"`
	Message     string           `json:"message,omitempty"`
	Facts       []poc.Fact       `json:"facts,omitempty"`
	Suggestions []poc.Suggestion `json:"suggestions,omitempty"`
}

type ConnectionState struct {
	Connected bool     `json:"connected"`
	Selected  Endpoint `json:"selected"`

	Addr         string `json:"addr,omitempty"`
	SystemAddr   string `json:"system_addr,omitempty"`
	UserAddr     string `json:"user_addr,omitempty"`
	OverrideAddr string `json:"override_addr,omitempty"`

	Failure *BridgeError `json:"failure,omitempty"`
}
