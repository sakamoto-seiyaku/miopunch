package poc

// Stage is a stable progress identifier used by the POC stage machine.
type Stage string

const (
	StageControlPlaneReady   Stage = "ControlPlaneReady"
	StageSelfDiscovery       Stage = "SelfDiscovery"
	StagePeerContact         Stage = "PeerContact"
	StageCandidateExchange   Stage = "CandidateExchange"
	StagePunchAttempt        Stage = "PunchAttempt"
	StageDataplaneHandshake  Stage = "DataplaneHandshake"
	StageCapabilityHandshake Stage = "CapabilityHandshake"
	StageSessionAttach       Stage = "SessionAttach"
)

var stageSet = map[Stage]struct{}{
	StageControlPlaneReady:   {},
	StageSelfDiscovery:       {},
	StagePeerContact:         {},
	StageCandidateExchange:   {},
	StagePunchAttempt:        {},
	StageDataplaneHandshake:  {},
	StageCapabilityHandshake: {},
	StageSessionAttach:       {},
}

// IsValidStage reports whether s is one of the fixed POC stage identifiers.
func IsValidStage(s Stage) bool {
	_, ok := stageSet[s]
	return ok
}
