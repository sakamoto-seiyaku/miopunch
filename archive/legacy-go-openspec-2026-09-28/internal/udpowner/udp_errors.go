package udpowner

import (
	"errors"
	"net"
)

func recoverableUDPReadError(err error) bool {
	if err == nil {
		return false
	}
	if udpReadTimeoutError(err) {
		return true
	}
	var netErr net.Error
	if errors.As(err, &netErr) && netErr.Temporary() {
		return true
	}
	return recoverableSocketReadError(err)
}

func udpReadTimeoutError(err error) bool {
	var netErr net.Error
	return errors.As(err, &netErr) && netErr.Timeout()
}
