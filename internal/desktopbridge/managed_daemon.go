package desktopbridge

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sync"
	"time"

	"github.com/miopunch/miopunch/internal/bundlepath"
)

const daemonOutputLimit = 16 * 1024

// ManagedDaemon is a miopunch up process started and owned by the desktop app.
type ManagedDaemon struct {
	cmd    *exec.Cmd
	done   chan error
	stdout *limitedBuffer
	stderr *limitedBuffer

	mu     sync.Mutex
	exited bool
	err    error
}

// StartManagedDaemon starts the sibling miopunch daemon in foreground session mode.
func StartManagedDaemon(daemonPath string) (*ManagedDaemon, error) {
	if daemonPath == "" {
		return nil, fmt.Errorf("empty daemon path")
	}

	stdout := newLimitedBuffer(daemonOutputLimit)
	stderr := newLimitedBuffer(daemonOutputLimit)
	args, err := managedDaemonArgs(daemonPath)
	if err != nil {
		return nil, err
	}
	cmd := exec.Command(daemonPath, args...)
	cmd.Dir = filepath.Dir(daemonPath)
	cmd.Stdout = stdout
	cmd.Stderr = stderr
	configureManagedDaemonCmd(cmd)

	if err := cmd.Start(); err != nil {
		return nil, fmt.Errorf("start miopunch up: %w", err)
	}

	d := &ManagedDaemon{
		cmd:    cmd,
		done:   make(chan error, 1),
		stdout: stdout,
		stderr: stderr,
	}
	go func() {
		err := cmd.Wait()
		d.mu.Lock()
		d.exited = true
		d.err = err
		d.mu.Unlock()
		d.done <- err
		close(d.done)
	}()
	return d, nil
}

func managedDaemonArgs(daemonPath string) ([]string, error) {
	statePath, err := bundlepath.StatePathForExecutable(daemonPath)
	if err != nil {
		return nil, fmt.Errorf("resolve session state path: %w", err)
	}
	return []string{"up", "--session", "--state_path", statePath}, nil
}

// PID returns the managed daemon process ID when available.
func (d *ManagedDaemon) PID() int {
	if d == nil || d.cmd == nil || d.cmd.Process == nil {
		return 0
	}
	return d.cmd.Process.Pid
}

// Stdout returns captured daemon stdout.
func (d *ManagedDaemon) Stdout() string {
	if d == nil || d.stdout == nil {
		return ""
	}
	return d.stdout.String()
}

// Stderr returns captured daemon stderr.
func (d *ManagedDaemon) Stderr() string {
	if d == nil || d.stderr == nil {
		return ""
	}
	return d.stderr.String()
}

// Err returns a daemon exit error if the process has already exited.
func (d *ManagedDaemon) Err() error {
	err, exited := d.Exited()
	if !exited {
		return nil
	}
	return err
}

// Exited reports whether the daemon has exited and returns its wait error.
func (d *ManagedDaemon) Exited() (error, bool) {
	if d == nil || d.done == nil {
		return nil, false
	}

	d.mu.Lock()
	defer d.mu.Unlock()
	return d.err, d.exited
}

// Stop best-effort stops the managed daemon and waits for it to exit.
func (d *ManagedDaemon) Stop(ctx context.Context) error {
	if d == nil || d.cmd == nil || d.cmd.Process == nil {
		return nil
	}
	if ctx == nil {
		ctx = context.Background()
	}

	if err, exited := d.Exited(); exited {
		return err
	}

	if runtime.GOOS != "windows" {
		_ = d.cmd.Process.Signal(os.Interrupt)
	}

	select {
	case err := <-d.done:
		return err
	case <-ctx.Done():
		_ = d.cmd.Process.Kill()
		select {
		case err := <-d.done:
			return err
		case <-time.After(2 * time.Second):
			return ctx.Err()
		}
	}
}

func resolveSiblingDaemonPath() (string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", fmt.Errorf("resolve desktop executable: %w", err)
	}

	name := "miopunch"
	if runtime.GOOS == "windows" {
		name += ".exe"
	}

	path := filepath.Join(filepath.Dir(exe), name)
	info, err := os.Stat(path)
	if err != nil {
		return "", fmt.Errorf("session bundle is missing sibling daemon %q: %w", path, err)
	}
	if info.IsDir() {
		return "", fmt.Errorf("session bundle daemon path is a directory: %s", path)
	}
	if runtime.GOOS != "windows" && info.Mode()&0o111 == 0 {
		return "", fmt.Errorf("session bundle daemon is not executable: %s", path)
	}
	return path, nil
}

type limitedBuffer struct {
	mu  sync.Mutex
	max int
	buf []byte
}

func newLimitedBuffer(max int) *limitedBuffer {
	return &limitedBuffer{max: max}
}

func (b *limitedBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()

	b.buf = append(b.buf, p...)
	if b.max > 0 && len(b.buf) > b.max {
		b.buf = append([]byte(nil), b.buf[len(b.buf)-b.max:]...)
	}
	return len(p), nil
}

func (b *limitedBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return string(b.buf)
}
