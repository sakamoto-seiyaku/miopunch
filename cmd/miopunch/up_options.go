package main

import (
	"errors"
	"fmt"
	"strings"

	"github.com/miopunch/miopunch/internal/http_panel"
)

type upOptions struct {
	HTTPPanel           bool
	HTTPPanelListenAddr string
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

	return opt, rest, nil
}
