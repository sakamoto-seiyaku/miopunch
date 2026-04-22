package http_panel

import (
	"fmt"
	"net"
	"strings"
)

const DefaultListenAddr = "127.0.0.1:27400"

type ListenAddrError struct {
	ListenAddr string
	Problem    string
}

func (e *ListenAddrError) Error() string {
	return fmt.Sprintf("invalid listen_addr %q: %s", e.ListenAddr, e.Problem)
}

func Listen(listenAddr string) (net.Listener, string, error) {
	addr := strings.TrimSpace(listenAddr)
	if addr == "" {
		addr = DefaultListenAddr
	}

	host, port, err := net.SplitHostPort(addr)
	if err != nil {
		return nil, "", &ListenAddrError{ListenAddr: addr, Problem: err.Error()}
	}
	if strings.TrimSpace(host) != "127.0.0.1" {
		return nil, "", &ListenAddrError{
			ListenAddr: addr,
			Problem:    "must listen on 127.0.0.1 (loopback-only)",
		}
	}

	ln, err := net.Listen("tcp4", net.JoinHostPort(host, port))
	if err != nil {
		return nil, "", err
	}

	return ln, ln.Addr().String(), nil
}
