package localapi

import "github.com/miopunch/miopunch/internal/poc"

// ErrorResponse is the minimum LocalAPI error response body.
type ErrorResponse struct {
	Stage       string           `json:"stage"`
	ReasonCode  poc.ReasonCode   `json:"reason_code"`
	ExitCode    poc.ExitCode     `json:"exit_code"`
	Message     string           `json:"message"`
	Facts       []poc.Fact       `json:"facts"`
	Suggestions []poc.Suggestion `json:"suggestions"`
	RequestID   string           `json:"request_id"`
}
