//go:build !windows

package localapi

import (
	"errors"
	"fmt"
	"net"
	"os"
	"os/user"
	"path/filepath"
	"strconv"
)

func systemUnixSocketPath() string {
	return "/run/miopunch/localapi.sock"
}

func userUnixSocketPath() (string, error) {
	dir := os.Getenv("XDG_RUNTIME_DIR")
	if dir == "" {
		return "", errors.New("XDG_RUNTIME_DIR is not set")
	}
	return filepath.Join(dir, "miopunch", "localapi.sock"), nil
}

type unixListenOptions struct {
	socketPath string
	dirMode    os.FileMode
	socketMode os.FileMode
	groupName  string
}

func listenUnixSocket(opt unixListenOptions) (net.Listener, error) {
	if opt.socketPath == "" {
		return nil, errors.New("empty socket path")
	}
	dir := filepath.Dir(opt.socketPath)
	if err := os.MkdirAll(dir, opt.dirMode); err != nil {
		return nil, fmt.Errorf("mkdir socket dir: %w", err)
	}
	if err := os.Chmod(dir, opt.dirMode); err != nil {
		return nil, fmt.Errorf("chmod socket dir: %w", err)
	}

	var gid int
	if opt.groupName != "" {
		g, err := user.LookupGroup(opt.groupName)
		if err != nil {
			return nil, fmt.Errorf("lookup group %q: %w", opt.groupName, err)
		}
		gid64, err := strconv.ParseInt(g.Gid, 10, 32)
		if err != nil {
			return nil, fmt.Errorf("parse group gid: %w", err)
		}
		gid = int(gid64)

		// Best-effort: apply group ownership on the directory.
		_ = os.Chown(dir, -1, gid)
	}

	ln, err := net.Listen("unix", opt.socketPath)
	if err != nil {
		return nil, err
	}

	if opt.socketMode != 0 {
		_ = os.Chmod(opt.socketPath, opt.socketMode)
	}
	if opt.groupName != "" {
		_ = os.Chown(opt.socketPath, -1, gid)
	}

	return ln, nil
}
