//go:build windows

package shelltarget

import (
	"context"
	"errors"
	"fmt"
	"os/exec"
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
		cmd = exec.CommandContext(ctx, "wsl.exe", windowsWSLListSessionsArgs(name)...)
	case "ssh":
		if _, err := exec.LookPath("ssh"); err != nil {
			return nil, err
		}
		cmd = exec.CommandContext(ctx, "ssh", windowsSSHListSessionsArgs(name)...)
	default:
		return nil, TargetNotFoundError{Input: target, Targets: []string{}}
	}

	out, runErr := cmd.CombinedOutput()
	if runErr != nil {
		msg := string(out)
		if looksLikeNoTmuxServer(msg) {
			return []string{}, nil
		}
		if looksLikeTmuxMissing(msg) {
			return nil, ErrTmuxMissing
		}
		return nil, fmt.Errorf("tmux list-sessions: %w: %s", runErr, strings.TrimSpace(msg))
	}

	if kind == "wsl" {
		return parseDefaultTmuxSessionNames(out), nil
	}
	return parsePlainTmuxSessionNames(out), nil
}

func ProbeReadiness(ctx context.Context, target string) (TargetReadiness, error) {
	target = strings.TrimSpace(target)
	if target == "" {
		return TargetReadiness{}, TargetNotFoundError{Input: target, Targets: []string{}}
	}
	kind, name, err := parseWindowsTarget(target)
	if err != nil {
		return TargetReadiness{}, err
	}

	var cmd *exec.Cmd
	switch kind {
	case "wsl":
		if _, err := exec.LookPath("wsl.exe"); err != nil {
			return classifyTargetReadiness(target, err, ""), nil
		}
		cmd = exec.CommandContext(ctx, "wsl.exe", windowsWSLPreflightTmuxArgs(name)...)
	case "ssh":
		if _, err := exec.LookPath("ssh"); err != nil {
			return classifyTargetReadiness(target, err, ""), nil
		}
		cmd = exec.CommandContext(ctx, "ssh", windowsSSHReadyProbeArgs(name)...)
	default:
		return TargetReadiness{}, TargetNotFoundError{Input: target, Targets: []string{}}
	}

	out, runErr := cmd.CombinedOutput()
	return classifyTargetReadiness(target, runErr, string(out)), nil
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
		if err := ensureWindowsTargetTmux(ctx, kind, name); err != nil {
			return nil, err
		}
		return startConPTY("wsl.exe", windowsWSLAttachArgs(name, session), 80, 24)
	case "ssh":
		if _, err := exec.LookPath("ssh"); err != nil {
			return nil, err
		}
		if err := ensureWindowsTargetTmux(ctx, kind, name); err != nil {
			return nil, err
		}
		return startConPTY("ssh", windowsSSHAttachArgs(name, session), 80, 24)
	default:
		return nil, TargetNotFoundError{Input: target, Targets: []string{}}
	}
}

func ensureWindowsTargetTmux(ctx context.Context, kind string, name string) error {
	var cmd *exec.Cmd
	switch kind {
	case "wsl":
		cmd = exec.CommandContext(ctx, "wsl.exe", windowsWSLPreflightTmuxArgs(name)...)
	case "ssh":
		cmd = exec.CommandContext(ctx, "ssh", windowsSSHPreflightTmuxArgs(name)...)
	default:
		return TargetNotFoundError{Input: kind + ":" + name, Targets: []string{}}
	}

	out, err := cmd.CombinedOutput()
	if err == nil {
		return nil
	}
	msg := strings.TrimSpace(string(out))
	if looksLikeTmuxMissing(msg) {
		return ErrTmuxMissing
	}
	if msg == "" {
		return fmt.Errorf("%s tmux preflight: %w", kind, err)
	}
	return fmt.Errorf("%s tmux preflight: %w: %s", kind, err, msg)
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
