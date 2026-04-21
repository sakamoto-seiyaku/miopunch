//go:build windows

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
	if target == "" {
		return nil, TargetNotFoundError{Input: target, Targets: []string{}}
	}

	kind, name, err := parseWindowsTarget(target)
	if err != nil {
		return nil, err
	}

	var cmd *exec.Cmd
	switch kind {
	case "wsl":
		if _, err := exec.LookPath("wsl.exe"); err != nil {
			return nil, err
		}
		cmd = exec.CommandContext(ctx, "wsl.exe", "-d", name, "--", "tmux", "list-sessions", "-F", "#S")
	case "ssh":
		if _, err := exec.LookPath("ssh"); err != nil {
			return nil, err
		}
		cmd = exec.CommandContext(ctx, "ssh", name, "--", "tmux", "list-sessions", "-F", "#S")
	default:
		return nil, TargetNotFoundError{Input: target, Targets: []string{}}
	}

	out, runErr := cmd.CombinedOutput()
	if runErr != nil {
		msg := string(out)
		if strings.Contains(msg, "failed to connect to server") ||
			strings.Contains(msg, "no server running") {
			return []string{}, nil
		}
		if looksLikeTmuxMissing(msg) {
			return nil, ErrTmuxMissing
		}
		return nil, fmt.Errorf("tmux list-sessions: %w: %s", runErr, strings.TrimSpace(msg))
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
	if target == "" {
		return nil, TargetNotFoundError{Input: target, Targets: []string{}}
	}

	kind, name, err := parseWindowsTarget(target)
	if err != nil {
		return nil, err
	}

	session = strings.TrimSpace(session)
	if session == "" {
		session = "main"
	}

	switch kind {
	case "wsl":
		if _, err := exec.LookPath("wsl.exe"); err != nil {
			return nil, err
		}
		return startConPTY("wsl.exe", []string{"-d", name, "--", "tmux", "new", "-A", "-s", session}, 80, 24)
	case "ssh":
		if _, err := exec.LookPath("ssh"); err != nil {
			return nil, err
		}
		return startConPTY("ssh", []string{"-tt", name, "tmux", "new", "-A", "-s", session}, 80, 24)
	default:
		return nil, TargetNotFoundError{Input: target, Targets: []string{}}
	}
}

func parseWindowsTarget(target string) (kind string, name string, err error) {
	if target == "local" {
		return "", "", TargetNotFoundError{Input: target, Targets: []string{}}
	}
	if strings.HasPrefix(target, "wsl:") {
		name = strings.TrimPrefix(target, "wsl:")
		name = strings.TrimSpace(name)
		if name == "" {
			return "", "", errors.New("empty wsl distro")
		}
		return "wsl", name, nil
	}
	if strings.HasPrefix(target, "ssh:") {
		name = strings.TrimPrefix(target, "ssh:")
		name = strings.TrimSpace(name)
		if name == "" {
			return "", "", errors.New("empty ssh name")
		}
		return "ssh", name, nil
	}
	return "", "", TargetNotFoundError{Input: target, Targets: []string{}}
}

func looksLikeTmuxMissing(out string) bool {
	out = strings.ToLower(out)
	return strings.Contains(out, "tmux: not found") ||
		strings.Contains(out, "tmux: command not found") ||
		strings.Contains(out, "'tmux' is not recognized")
}
