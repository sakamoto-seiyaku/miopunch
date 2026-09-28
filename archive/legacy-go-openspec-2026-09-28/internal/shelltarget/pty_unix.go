//go:build !windows

package shelltarget

import (
	"os"
	"os/exec"

	"github.com/creack/pty"
)

type unixPTY struct {
	f   *os.File
	cmd *exec.Cmd
}

func startUnixPTY(cmd *exec.Cmd) (*unixPTY, error) {
	f, err := pty.Start(cmd)
	if err != nil {
		return nil, err
	}
	return &unixPTY{f: f, cmd: cmd}, nil
}

func (p *unixPTY) Read(b []byte) (int, error)  { return p.f.Read(b) }
func (p *unixPTY) Write(b []byte) (int, error) { return p.f.Write(b) }

func (p *unixPTY) Resize(cols, rows int) error {
	if p == nil || p.f == nil {
		return nil
	}
	if cols <= 0 || rows <= 0 {
		return nil
	}
	return pty.Setsize(p.f, &pty.Winsize{
		Cols: uint16(cols),
		Rows: uint16(rows),
	})
}

func (p *unixPTY) Wait() error {
	if p == nil || p.cmd == nil {
		return nil
	}
	return p.cmd.Wait()
}

func (p *unixPTY) Close() error {
	if p == nil {
		return nil
	}
	if p.f != nil {
		_ = p.f.Close()
	}
	return nil
}
