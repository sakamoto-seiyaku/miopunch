//go:build windows

package shelltarget

import (
	"bufio"
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
)

func ListTargets(ctx context.Context) ([]string, error) {
	targets := make([]string, 0, 8)

	distros, _ := listWSLDistros(ctx)
	for _, d := range distros {
		targets = append(targets, "wsl:"+d)
	}

	hosts, _ := listSSHHosts()
	for _, h := range hosts {
		targets = append(targets, "ssh:"+h)
	}

	seen := map[string]struct{}{}
	uniq := make([]string, 0, len(targets))
	for _, t := range targets {
		t = strings.TrimSpace(t)
		if t == "" {
			continue
		}
		if _, ok := seen[t]; ok {
			continue
		}
		seen[t] = struct{}{}
		uniq = append(uniq, t)
	}
	sort.Strings(uniq)
	return uniq, nil
}

func listWSLDistros(ctx context.Context) ([]string, error) {
	if _, err := exec.LookPath("wsl.exe"); err != nil {
		return nil, err
	}

	cmd := exec.CommandContext(ctx, "wsl.exe", "-l", "-q")
	out, err := cmd.Output()
	if err != nil {
		return nil, err
	}

	sc := bufio.NewScanner(strings.NewReader(decodeWindowsCommandOutput(out)))
	var distros []string
	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		if line == "" {
			continue
		}
		distros = append(distros, line)
	}
	if err := sc.Err(); err != nil {
		return nil, err
	}
	sort.Strings(distros)
	return distros, nil
}

func listSSHHosts() ([]string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, err
	}
	cfgPath := filepath.Join(home, ".ssh", "config")
	f, err := os.Open(cfgPath)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return []string{}, nil
		}
		return nil, err
	}
	defer func() { _ = f.Close() }()

	seen := map[string]struct{}{}
	var hosts []string

	sc := bufio.NewScanner(f)
	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}

		lower := strings.ToLower(line)
		if !strings.HasPrefix(lower, "host ") && !strings.HasPrefix(lower, "host\t") {
			continue
		}

		fields := strings.Fields(line)
		if len(fields) < 2 {
			continue
		}
		for _, pattern := range fields[1:] {
			pattern = strings.TrimSpace(pattern)
			if pattern == "" || strings.ContainsAny(pattern, "*?") || strings.HasPrefix(pattern, "!") {
				continue
			}
			if _, ok := seen[pattern]; ok {
				continue
			}
			seen[pattern] = struct{}{}
			hosts = append(hosts, pattern)
		}
	}
	if err := sc.Err(); err != nil {
		return nil, err
	}
	sort.Strings(hosts)
	return hosts, nil
}
