//go:build !aix && !android && !darwin && !dragonfly && !freebsd && !linux && !netbsd && !openbsd && !solaris && !windows

package udpowner

func recoverableSocketReadError(error) bool {
	return false
}
