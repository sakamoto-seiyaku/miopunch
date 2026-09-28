package main

import (
	"encoding/json"
	"fmt"
	"io"
	"strings"

	"github.com/miopunch/miopunch/internal/poc"
)

type failureOutput struct {
	Stage       string
	ReasonCode  poc.ReasonCode
	ExitCode    poc.ExitCode
	Facts       []poc.Fact
	Suggestions []poc.Suggestion
}

func writeFailure(w io.Writer, f failureOutput) {
	fmt.Fprintf(w, "stage=%s\n", f.Stage)
	fmt.Fprintf(w, "reason_code=%s\n", f.ReasonCode)
	fmt.Fprintf(w, "exit_code=%d\n", f.ExitCode)
	fmt.Fprintln(w, "facts:")
	for _, fact := range f.Facts {
		msg := strings.TrimSpace(fact.Message)
		if msg == "" {
			continue
		}
		fmt.Fprintf(w, "- %s\n", msg)
	}
	fmt.Fprintln(w, "suggestions:")
	for _, s := range f.Suggestions {
		msg := strings.TrimSpace(s.Message)
		if msg == "" {
			continue
		}
		fmt.Fprintf(w, "- %s\n", msg)
	}
}

func writeEnvelopeJSON(w io.Writer, env poc.EnvelopeJSONV0) {
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	_ = enc.Encode(env)
}
