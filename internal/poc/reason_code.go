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

	// POC-06: shell / tmux vertical slice diagnostics (POC v0).
	ReasonCodeSHTargetNotFound  ReasonCode = "SH_TARGET_NOT_FOUND"
	ReasonCodeSHTargetAmbiguous ReasonCode = "SH_TARGET_AMBIGUOUS"
	ReasonCodeSHInUse           ReasonCode = "SH_IN_USE"
	ReasonCodeSHConnectorFail   ReasonCode = "SH_CONNECTOR_FAIL"
	ReasonCodeSHTmuxMissing     ReasonCode = "SH_TMUX_MISSING"
	ReasonCodeSHTmuxAttachFail  ReasonCode = "SH_TMUX_ATTACH_FAIL"
)
