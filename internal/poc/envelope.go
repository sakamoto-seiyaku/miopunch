package poc

const JSONFormatV0 = "miopunch.json.v0"

// EnvelopeJSONV0 is the minimum stable top-level JSON envelope for `--format json`.
//
// NOTE: Only the top-level field names and basic types are frozen for POC.
// Sub-fields may evolve, and additional fields may be added.
type EnvelopeJSONV0 struct {
	Format      string       `json:"format"`
	TaskID      string       `json:"task_id"`
	Kind        string       `json:"kind"`
	Status      string       `json:"status"`
	Stage       string       `json:"stage"`
	ReasonCode  ReasonCode   `json:"reason_code,omitempty"`
	ExitCode    ExitCode     `json:"exit_code,omitempty"`
	Facts       []Fact       `json:"facts"`
	Suggestions []Suggestion `json:"suggestions"`
}

func NewEnvelopeJSONV0() EnvelopeJSONV0 {
	return EnvelopeJSONV0{
		Format:      JSONFormatV0,
		Facts:       []Fact{},
		Suggestions: []Suggestion{},
	}
}
