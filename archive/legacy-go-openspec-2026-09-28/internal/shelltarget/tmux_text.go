package shelltarget

import (
	"sort"
	"strings"
)

func looksLikeTmuxMissing(out string) bool {
	out = strings.ToLower(out)
	return strings.Contains(out, "tmux: not found") ||
		strings.Contains(out, "tmux: command not found") ||
		strings.Contains(out, "command not found: tmux") ||
		strings.Contains(out, "'tmux' is not recognized")
}

func looksLikeNoTmuxServer(out string) bool {
	out = strings.ToLower(out)
	return strings.Contains(out, "failed to connect to server") ||
		strings.Contains(out, "no server running") ||
		(strings.Contains(out, "error connecting to ") &&
			strings.Contains(out, "no such file or directory"))
}

func looksLikeTimeout(out string) bool {
	out = strings.ToLower(out)
	return strings.Contains(out, "connection timed out") ||
		strings.Contains(out, "operation timed out") ||
		strings.Contains(out, "i/o timeout")
}

func parsePlainTmuxSessionNames(out []byte) []string {
	return collectTmuxSessionNames(out, func(line string) string {
		return line
	})
}

func parseDefaultTmuxSessionNames(out []byte) []string {
	if looksLikeNoTmuxServer(string(out)) {
		return []string{}
	}
	return collectTmuxSessionNames(out, func(line string) string {
		name, _, _ := strings.Cut(line, ":")
		return name
	})
}

func collectTmuxSessionNames(out []byte, parse func(string) string) []string {
	lines := strings.Split(string(out), "\n")
	seen := make(map[string]struct{}, len(lines))
	sessions := make([]string, 0, len(lines))
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" || looksLikeNoTmuxServer(line) {
			continue
		}
		session := strings.TrimSpace(parse(line))
		if session == "" {
			continue
		}
		if _, ok := seen[session]; ok {
			continue
		}
		seen[session] = struct{}{}
		sessions = append(sessions, session)
	}
	sort.Strings(sessions)
	return sessions
}
