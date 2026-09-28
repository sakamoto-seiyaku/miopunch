package connectivity

import (
	"context"
	"errors"
	"net"
	"time"
)

func listenTCPWithReuseAddr(network, address string) (*net.TCPListener, error) {
	lc := net.ListenConfig{Control: tcpReuseControl}
	ln, err := lc.Listen(context.Background(), network, address)
	if err != nil {
		return nil, err
	}
	tl, ok := ln.(*net.TCPListener)
	if !ok {
		_ = ln.Close()
		return nil, errors.New("listener is not TCP")
	}
	return tl, nil
}

func newTCPDialerWithReuseAddr(localAddr *net.TCPAddr, timeout time.Duration) *net.Dialer {
	dialer := &net.Dialer{
		Control:   tcpReuseControl,
		Timeout:   timeout,
		LocalAddr: localAddr,
	}
	return dialer
}
