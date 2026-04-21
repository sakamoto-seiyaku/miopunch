package localapi

import "fmt"

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

type ListenMode string

const (
	ListenModeSystem ListenMode = "system"
	ListenModeUser   ListenMode = "user"
)
