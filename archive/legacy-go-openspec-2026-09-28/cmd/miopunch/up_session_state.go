package main

import (
	"fmt"
	"strings"

	"github.com/miopunch/miopunch/internal/bundlepath"
	"github.com/miopunch/miopunch/internal/poc"
)

func applySessionStatePath(opt upOptions) (upOptions, error) {
	return applySessionStatePathWithResolver(opt, bundlepath.StatePath)
}

func applySessionStatePathWithResolver(opt upOptions, resolve func() (string, error)) (upOptions, error) {
	if !opt.Session || strings.TrimSpace(opt.StatePath) != "" {
		return opt, nil
	}
	if resolve == nil {
		return opt, fmt.Errorf("nil session state path resolver")
	}

	statePath, err := resolve()
	if err != nil {
		return opt, err
	}
	opt.StatePath = statePath
	return opt, nil
}

func sessionStatePathFailure(err error) failureOutput {
	msg := "failed to resolve portable session state path"
	if err != nil {
		msg += ": " + err.Error()
	}
	return failureOutput{
		Stage:      "daemon",
		ReasonCode: poc.ReasonCodeUnavailable,
		ExitCode:   poc.ExitCodeUnavailable,
		Facts: []poc.Fact{
			{Message: msg},
		},
		Suggestions: []poc.Suggestion{
			{Message: "extract the session bundle into a writable directory and retry"},
			{Message: "or pass --state_path <path> explicitly"},
		},
	}
}
