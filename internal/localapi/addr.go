package localapi

import (
	"fmt"
	"strings"
)

type Transport string

const (
	TransportUnix  Transport = "unix"
	TransportNpipe Transport = "npipe"
)

type Addr struct {
	Transport Transport
	Path      string

	// Windows-only (npipe): operator user SID used to scope the pipe name and ACL.
	OperatorSID string
}

func (a Addr) String() string {
	switch a.Transport {
	case TransportUnix:
		return "unix:" + a.Path
	case TransportNpipe:
		return "npipe:" + a.Path
	default:
		return fmt.Sprintf("%s:%s", a.Transport, a.Path)
	}
}

func ParseAddr(value string) (Addr, error) {
	v := strings.TrimSpace(value)
	switch {
	case strings.HasPrefix(v, "unix:"):
		path := strings.TrimSpace(strings.TrimPrefix(v, "unix:"))
		if path == "" {
			return Addr{}, fmt.Errorf("empty unix socket path")
		}
		return Addr{Transport: TransportUnix, Path: path}, nil
	case strings.HasPrefix(v, "npipe:"):
		path := strings.TrimSpace(strings.TrimPrefix(v, "npipe:"))
		if path == "" {
			return Addr{}, fmt.Errorf("empty npipe path")
		}
		return Addr{Transport: TransportNpipe, Path: path}, nil
	default:
		return Addr{}, fmt.Errorf("unsupported addr format: %q", value)
	}
}

type ListenMode string

const (
	ListenModeSystem ListenMode = "system"
	ListenModeUser   ListenMode = "user"
)
