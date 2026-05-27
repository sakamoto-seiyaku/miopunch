package localapi

import (
	"fmt"
	"strings"

	"github.com/miopunch/miopunch/internal/poc"
)

// ErrorResponse is the minimum structured LocalAPI error body.
type ErrorResponse struct {
	Stage       string           `json:"stage"`
	ReasonCode  poc.ReasonCode   `json:"reason_code"`
	ExitCode    poc.ExitCode     `json:"exit_code"`
	Message     string           `json:"message,omitempty"`
	Facts       []poc.Fact       `json:"facts"`
	Suggestions []poc.Suggestion `json:"suggestions"`
	RequestID   string           `json:"request_id,omitempty"`
}

type APIError struct {
	Response ErrorResponse
}

func (e *APIError) Error() string {
	if e == nil {
		return ""
	}
	msg := strings.TrimSpace(e.Response.Message)
	if msg == "" {
		msg = "localapi error"
	}
	return fmt.Sprintf(
		"%s (exit_code=%d reason_code=%s stage=%s)",
		msg,
		e.Response.ExitCode,
		e.Response.ReasonCode,
		e.Response.Stage,
	)
}

type UnexpectedStatusError struct {
	StatusCode int
	Problem    string
}

func (e *UnexpectedStatusError) Error() string {
	if e == nil {
		return ""
	}
	if strings.TrimSpace(e.Problem) == "" {
		return fmt.Sprintf("unexpected localapi status: %d", e.StatusCode)
	}
	return fmt.Sprintf("unexpected localapi status: %s", e.Problem)
}
