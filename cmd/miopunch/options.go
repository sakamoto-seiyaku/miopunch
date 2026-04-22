package main

import (
	"fmt"
	"strings"
)

type outputFormat string

const (
	outputFormatHuman outputFormat = "human"
	outputFormatJSON  outputFormat = "json"
)

type globalOptions struct {
	Format           outputFormat
	LocalAPIOverride string
	ReportPath       string
	Redact           bool
}

func parseGlobalOptions(args []string) (globalOptions, []string, error) {
	opt := globalOptions{Format: outputFormatHuman}

	i := 0
	for i < len(args) {
		a := args[i]
		if a == "--" {
			i++
			break
		}
		if !strings.HasPrefix(a, "-") {
			break
		}

		switch {
		case a == "--format":
			if i+1 >= len(args) {
				return globalOptions{}, nil, fmt.Errorf("missing value for --format")
			}
			i++
			if err := opt.setFormat(args[i]); err != nil {
				return globalOptions{}, nil, err
			}
			i++
		case strings.HasPrefix(a, "--format="):
			if err := opt.setFormat(strings.TrimPrefix(a, "--format=")); err != nil {
				return globalOptions{}, nil, err
			}
			i++
		case a == "--localapi":
			if i+1 >= len(args) {
				return globalOptions{}, nil, fmt.Errorf("missing value for --localapi")
			}
			i++
			opt.LocalAPIOverride = strings.TrimSpace(args[i])
			i++
		case strings.HasPrefix(a, "--localapi="):
			opt.LocalAPIOverride = strings.TrimSpace(strings.TrimPrefix(a, "--localapi="))
			i++
		case a == "--report":
			if i+1 >= len(args) {
				return globalOptions{}, nil, fmt.Errorf("missing value for --report")
			}
			i++
			opt.ReportPath = strings.TrimSpace(args[i])
			i++
		case strings.HasPrefix(a, "--report="):
			opt.ReportPath = strings.TrimSpace(strings.TrimPrefix(a, "--report="))
			i++
		case a == "--redact":
			opt.Redact = true
			i++
		case a == "--redact=true":
			opt.Redact = true
			i++
		case a == "--redact=false":
			opt.Redact = false
			i++
		default:
			// Stop parsing flags at the first unrecognized flag. This keeps
			// subcommand-specific flags forward-compatible.
			return opt, args[i:], nil
		}
	}
	return opt, args[i:], nil
}

func (o *globalOptions) setFormat(value string) error {
	switch strings.TrimSpace(value) {
	case "", "human":
		o.Format = outputFormatHuman
		return nil
	case "json":
		o.Format = outputFormatJSON
		return nil
	default:
		return fmt.Errorf("invalid --format: %q", value)
	}
}
