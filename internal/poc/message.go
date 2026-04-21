package poc

// Fact is a concrete observation surfaced to the user.
type Fact struct {
	TermID  string `json:"term_id,omitempty"`
	Message string `json:"message"`
}

// Suggestion is a concrete user action suggestion.
type Suggestion struct {
	TermID  string `json:"term_id,omitempty"`
	Message string `json:"message"`
}
