package poc

// ReasonCode is a stable leaf diagnosis identifier.
type ReasonCode string

const (
	ReasonCodeOK             ReasonCode = "OK"
	ReasonCodeNotImplemented ReasonCode = "NOT_IMPLEMENTED"

	ReasonCodeDaemonNotRunning ReasonCode = "DAEMON_NOT_RUNNING"
	ReasonCodeBadRequest       ReasonCode = "BAD_REQUEST"
	ReasonCodeForbidden        ReasonCode = "FORBIDDEN"
	ReasonCodeConflict         ReasonCode = "CONFLICT"
	ReasonCodeNotFound         ReasonCode = "NOT_FOUND"
	ReasonCodeTimeout          ReasonCode = "TIMEOUT"
	ReasonCodeUnavailable      ReasonCode = "UNAVAILABLE"
	ReasonCodeInternal         ReasonCode = "INTERNAL"
)
