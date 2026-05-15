//go:build !windows

package shelltarget

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strings"
)

func ListSessions(ctx context.Context, target string) ([]string, error) {
	target = strings.TrimSpace(target)
	if target != "local" {
		return nil, TargetNotFoundError{Input: target, Targets: []string{"local"}}
	}

	if _, err := exec.LookPath("tmux"); err != nil {
		return nil, ErrTmuxMissing
	}

	cmd := exec.CommandContext(ctx, "tmux", "list-sessions", "-F", "#S")
	out, err := cmd.CombinedOutput()
	if err != nil {
		// tmux returns exit code 1 when no server is running (no sessions).
		msg := string(out)
		if looksLikeNoTmuxServer(msg) {
			return []string{}, nil
		}
		return nil, fmt.Errorf("tmux list-sessions: %w: %s", err, strings.TrimSpace(msg))
	}

	return parsePlainTmuxSessionNames(out), nil
}

func Attach(ctx context.Context, target string, session string) (PTY, error) {
	target = strings.TrimSpace(target)
	if target != "local" {
		return nil, TargetNotFoundError{Input: target, Targets: []string{"local"}}
	}
	session = strings.TrimSpace(session)
	if session == "" {
		session = "main"
	}

	if _, err := exec.LookPath("tmux"); err != nil {
		return nil, ErrTmuxMissing
	}

	cmd := exec.CommandContext(ctx, "tmux", "new", "-A", "-s", session)
	env := os.Environ()
	hasTerm := false
	for _, v := range env {
		if strings.HasPrefix(v, "TERM=") && strings.TrimSpace(strings.TrimPrefix(v, "TERM=")) != "" {
			hasTerm = true
			break
		}
	}
	if !hasTerm {
		env = append(env, "TERM=xterm-256color")
	}
	cmd.Env = env
	p, err := startUnixPTY(cmd)
	if err != nil {
		if errors.Is(err, exec.ErrNotFound) {
			return nil, ErrTmuxMissing
		}
		return nil, err
	}
	return p, nil
}
