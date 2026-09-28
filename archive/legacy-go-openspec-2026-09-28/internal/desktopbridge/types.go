package desktopbridge

import "github.com/miopunch/miopunch/internal/poc"

type Endpoint string

const (
	EndpointNone     Endpoint = "none"
	EndpointSystem   Endpoint = "system"
	EndpointUser     Endpoint = "user"
	EndpointOverride Endpoint = "override"
)

type BootstrapState string

const (
	BootstrapNone   BootstrapState = "none"
	BootstrapReady  BootstrapState = "ready"
	BootstrapFailed BootstrapState = "failed"
)

type BootstrapDiagnostics struct {
	Attempted  bool   `json:"attempted"`
	Stage      string `json:"stage,omitempty"`
	DaemonPath string `json:"daemon_path,omitempty"`
	ProbeAddr  string `json:"probe_addr,omitempty"`
	PID        int    `json:"pid,omitempty"`
	Stdout     string `json:"stdout,omitempty"`
	Stderr     string `json:"stderr,omitempty"`
	Error      string `json:"error,omitempty"`
}

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

	Bootstrap      BootstrapState        `json:"bootstrap_state,omitempty"`
	DesktopManaged bool                  `json:"desktop_managed,omitempty"`
	Diagnostics    []poc.Fact            `json:"diagnostics,omitempty"`
	BootstrapInfo  *BootstrapDiagnostics `json:"bootstrap,omitempty"`
	Failure        *BridgeError          `json:"failure,omitempty"`
}
