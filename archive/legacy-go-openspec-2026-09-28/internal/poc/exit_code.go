package poc

import "net/http"

// ExitCode is a coarse error category used by the POC output contract.
type ExitCode int

const (
	ExitCodeOK ExitCode = 0

	// ExitCodeInternal indicates an unexpected internal error.
	ExitCodeInternal ExitCode = 1

	// ExitCodeBadRequest indicates invalid inputs or request shape.
	ExitCodeBadRequest ExitCode = 2

	// ExitCodeUnavailable indicates dependency or service unavailability.
	ExitCodeUnavailable ExitCode = 3

	// ExitCodeForbidden indicates an authorization/permission failure.
	ExitCodeForbidden ExitCode = 4

	// ExitCodeTimeout indicates a deadline or timeout failure.
	ExitCodeTimeout ExitCode = 5

	// ExitCodeConflict indicates a concurrency or state conflict.
	ExitCodeConflict ExitCode = 6

	// ExitCodeNotFound indicates a missing resource.
	ExitCodeNotFound ExitCode = 7
)

// HTTPStatusFromExitCode maps POC exit codes to coarse HTTP statuses.
func HTTPStatusFromExitCode(exitCode ExitCode) int {
	switch exitCode {
	case ExitCodeBadRequest:
		return http.StatusBadRequest
	case ExitCodeUnavailable:
		return http.StatusServiceUnavailable
	case ExitCodeForbidden:
		return http.StatusForbidden
	case ExitCodeTimeout:
		return http.StatusGatewayTimeout
	case ExitCodeConflict:
		return http.StatusConflict
	case ExitCodeNotFound:
		return http.StatusNotFound
	case ExitCodeInternal:
		return http.StatusInternalServerError
	default:
		return http.StatusOK
	}
}
