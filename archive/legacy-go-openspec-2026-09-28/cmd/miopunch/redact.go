package main

import (
	"regexp"

	"github.com/miopunch/miopunch/internal/poc"
)

var redactRules = []struct {
	re   *regexp.Regexp
	repl string
}{
	{re: regexp.MustCompile(`(?m)(invite_code=)[^\s]+`), repl: `${1}<redacted>`},
	{re: regexp.MustCompile(`(?m)(secret_key=)[^\s]+`), repl: `${1}<redacted>`},
	{re: regexp.MustCompile(`(?m)(net_secret_b64=)[^\s]+`), repl: `${1}<redacted>`},
	{re: regexp.MustCompile(`(?m)(invite_secret_b64=)[^\s]+`), repl: `${1}<redacted>`},
}

func redactString(s string) string {
	out := s
	for _, rule := range redactRules {
		out = rule.re.ReplaceAllString(out, rule.repl)
	}
	return out
}

func redactFacts(in []poc.Fact) []poc.Fact {
	out := make([]poc.Fact, 0, len(in))
	for _, f := range in {
		f.Message = redactString(f.Message)
		out = append(out, f)
	}
	return out
}

func redactSuggestions(in []poc.Suggestion) []poc.Suggestion {
	out := make([]poc.Suggestion, 0, len(in))
	for _, s := range in {
		s.Message = redactString(s.Message)
		out = append(out, s)
	}
	return out
}
