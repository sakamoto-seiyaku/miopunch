package main

import (
	"errors"
	"fmt"
	"strings"

	"github.com/miopunch/miopunch/internal/http_panel"
	"github.com/miopunch/miopunch/internal/localapi"
)

type upOptions struct {
	HTTPPanel           bool
	HTTPPanelListenAddr string
	LocalAPIOverride    string
	BrokerOverride      string
	LogLevel            string
	Session             bool
	StatePath           string
}

func parseUpOptions(args []string) (upOptions, []string, error) {
	opt := upOptions{
		HTTPPanelListenAddr: http_panel.DefaultListenAddr,
	}

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
		case a == "--http_panel":
			opt.HTTPPanel = true
			i++
		case a == "--http_panel=true":
			opt.HTTPPanel = true
			i++
		case a == "--http_panel=false":
			opt.HTTPPanel = false
			i++
		case a == "--http_panel_listen_addr":
			if i+1 >= len(args) {
				return upOptions{}, nil, errors.New("missing value for --http_panel_listen_addr")
			}
			i++
			opt.HTTPPanelListenAddr = strings.TrimSpace(args[i])
			i++
		case strings.HasPrefix(a, "--http_panel_listen_addr="):
			opt.HTTPPanelListenAddr = strings.TrimSpace(strings.TrimPrefix(a, "--http_panel_listen_addr="))
			i++
		case a == "--localapi":
			if i+1 >= len(args) {
				return upOptions{}, nil, errors.New("missing value for --localapi")
			}
			i++
			opt.LocalAPIOverride = strings.TrimSpace(args[i])
			i++
		case strings.HasPrefix(a, "--localapi="):
			opt.LocalAPIOverride = strings.TrimSpace(strings.TrimPrefix(a, "--localapi="))
			i++
		case a == "--broker":
			if i+1 >= len(args) {
				return upOptions{}, nil, errors.New("missing value for --broker")
			}
			i++
			opt.BrokerOverride = strings.TrimSpace(args[i])
			i++
		case strings.HasPrefix(a, "--broker="):
			opt.BrokerOverride = strings.TrimSpace(strings.TrimPrefix(a, "--broker="))
			i++
		case a == "--log-level":
			if i+1 >= len(args) {
				return upOptions{}, nil, errors.New("missing value for --log-level")
			}
			i++
			opt.LogLevel = strings.ToLower(strings.TrimSpace(args[i]))
			i++
		case strings.HasPrefix(a, "--log-level="):
			opt.LogLevel = strings.ToLower(strings.TrimSpace(strings.TrimPrefix(a, "--log-level=")))
			i++
		case a == "--session":
			opt.Session = true
			i++
		case a == "--session=true":
			opt.Session = true
			i++
		case a == "--session=false":
			opt.Session = false
			i++
		case a == "--state_path":
			if i+1 >= len(args) {
				return upOptions{}, nil, errors.New("missing value for --state_path")
			}
			i++
			opt.StatePath = strings.TrimSpace(args[i])
			i++
		case strings.HasPrefix(a, "--state_path="):
			opt.StatePath = strings.TrimSpace(strings.TrimPrefix(a, "--state_path="))
			i++
		default:
			return upOptions{}, nil, fmt.Errorf("unknown flag: %s", a)
		}
	}

	rest := args[i:]
	if len(rest) != 0 {
		return upOptions{}, nil, errors.New("unexpected extra args")
	}

	if strings.TrimSpace(opt.HTTPPanelListenAddr) == "" {
		opt.HTTPPanelListenAddr = http_panel.DefaultListenAddr
	}
	if strings.TrimSpace(opt.LocalAPIOverride) != "" {
		if _, err := localapi.ParseAddr(opt.LocalAPIOverride); err != nil {
			return upOptions{}, nil, fmt.Errorf("invalid --localapi: %w", err)
		}
	}
	switch strings.TrimSpace(opt.LogLevel) {
	case "", "trace", "debug", "info", "warn", "error":
	default:
		return upOptions{}, nil, fmt.Errorf("invalid --log-level: %q", opt.LogLevel)
	}

	return opt, rest, nil
}
