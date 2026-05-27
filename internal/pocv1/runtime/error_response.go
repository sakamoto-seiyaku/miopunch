package runtime

import "github.com/miopunch/miopunch/internal/poc"

type ErrorResponse struct {
	Stage       string           `json:"stage"`
	ReasonCode  poc.ReasonCode   `json:"reason_code"`
	ExitCode    poc.ExitCode     `json:"exit_code"`
	Message     string           `json:"message,omitempty"`
	Facts       []poc.Fact       `json:"facts"`
	Suggestions []poc.Suggestion `json:"suggestions"`
}
