//go:build !windows

package shelltarget

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os/exec"
	"sort"
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
		if strings.Contains(msg, "failed to connect to server") ||
			strings.Contains(msg, "no server running") {
			return []string{}, nil
		}
		return nil, fmt.Errorf("tmux list-sessions: %w: %s", err, strings.TrimSpace(msg))
	}

	lines := bytes.Split(out, []byte("\n"))
	seen := map[string]struct{}{}
	sessions := make([]string, 0, len(lines))
	for _, line := range lines {
		s := strings.TrimSpace(string(line))
		if s == "" {
			continue
		}
		if _, ok := seen[s]; ok {
			continue
		}
		seen[s] = struct{}{}
		sessions = append(sessions, s)
	}
	sort.Strings(sessions)
	return sessions, nil
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
	p, err := startUnixPTY(cmd)
	if err != nil {
		if errors.Is(err, exec.ErrNotFound) {
			return nil, ErrTmuxMissing
		}
		return nil, err
	}
	return p, nil
}
