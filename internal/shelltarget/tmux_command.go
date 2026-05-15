package shelltarget

func windowsWSLListSessionsArgs(distro string) []string {
	return []string{"-d", distro, "--", "tmux", "list-sessions"}
}

func windowsSSHListSessionsArgs(host string) []string {
	return []string{host, "tmux", "list-sessions", "-F", "#S"}
}

func windowsWSLAttachArgs(distro string, session string) []string {
	return []string{"-d", distro, "--", "tmux", "new", "-A", "-s", session}
}

func windowsSSHAttachArgs(host string, session string) []string {
	return []string{"-tt", host, "tmux", "new", "-A", "-s", session}
}

func windowsWSLPreflightTmuxArgs(distro string) []string {
	return []string{"-d", distro, "--", "tmux", "-V"}
}

func windowsSSHPreflightTmuxArgs(host string) []string {
	return []string{host, "tmux", "-V"}
}
