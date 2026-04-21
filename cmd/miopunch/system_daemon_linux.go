//go:build linux

package main

import (
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"os/user"
	"path/filepath"
	"strings"

	"github.com/kardianos/service"

	"github.com/miopunch/miopunch/internal/poc"
)

const linuxStableBinaryPath = "/usr/local/bin/miopunch"

func runInstallSystemDaemon(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	_ = args
	_ = stdout

	if os.Geteuid() != 0 {
		return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeForbidden,
			ExitCode:   poc.ExitCodeForbidden,
			Facts: []poc.Fact{
				{Message: "install-system-daemon requires root"},
			},
			Suggestions: []poc.Suggestion{
				{Message: "run: sudo miopunch install-system-daemon"},
			},
		})
	}

	operatorUser, err := ensureLinuxOperatorAccess()
	if err != nil {
		facts := []poc.Fact{
			{Message: "failed to grant linux operator access"},
			{Message: "operator_group=" + poc.LinuxOperatorGroup},
			{Message: "error=" + err.Error()},
		}
		if operatorUser != "" {
			facts = append(facts, poc.Fact{Message: "operator_user=" + operatorUser})
		}
		return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts:      facts,
			Suggestions: []poc.Suggestion{
				{Message: "retry with root privileges"},
				{Message: "or create the group manually: sudo groupadd -f " + poc.LinuxOperatorGroup},
				{Message: "or grant access manually: sudo usermod -aG " + poc.LinuxOperatorGroup + " <user>"},
				{Message: "log out and log back in"},
			},
		})
	}

	if err := installStableBinary(linuxStableBinaryPath); err != nil {
		return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to install stable binary"},
				{Message: "path=" + linuxStableBinaryPath},
				{Message: "error=" + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
	}

	svc, err := linuxSystemService()
	if err != nil {
		return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to create service handle: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{{Message: "retry"}},
		})
	}

	if err := ensureServiceInstalled(svc); err != nil {
		return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to install system service: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{{Message: "retry"}},
		})
	}

	// Install acts as "upgrade" too: copy stable binary first, then restart.
	if err := svc.Restart(); err != nil {
		if err := svc.Start(); err != nil {
			return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeInternal,
				ExitCode:   poc.ExitCodeInternal,
				Facts: []poc.Fact{
					{Message: "failed to start service: " + err.Error()},
				},
				Suggestions: []poc.Suggestion{{Message: "retry"}},
			})
		}
	}

	fmt.Fprintln(stderr, "installed and started miopunch system daemon")
	fmt.Fprintf(stderr, "stable_binary=%s\n", linuxStableBinaryPath)
	fmt.Fprintf(stderr, "operator_group=%s\n", poc.LinuxOperatorGroup)
	fmt.Fprintf(stderr, "operator_user=%s\n", operatorUser)
	return 0
}

func runUninstallSystemDaemon(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	_ = args
	_ = stdout

	if os.Geteuid() != 0 {
		return exitWithFailure(opt, stdout, stderr, "uninstall-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeForbidden,
			ExitCode:   poc.ExitCodeForbidden,
			Facts: []poc.Fact{
				{Message: "uninstall-system-daemon requires root"},
			},
			Suggestions: []poc.Suggestion{
				{Message: "run: sudo miopunch uninstall-system-daemon"},
			},
		})
	}

	svc, err := linuxSystemService()
	if err != nil {
		return exitWithFailure(opt, stdout, stderr, "uninstall-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to create service handle: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{{Message: "retry"}},
		})
	}

	if _, err := svc.Status(); err != nil {
		if errors.Is(err, service.ErrNotInstalled) {
			return exitWithFailure(opt, stdout, stderr, "uninstall-system-daemon", "", failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeNotFound,
				ExitCode:   poc.ExitCodeNotFound,
				Facts: []poc.Fact{
					{Message: "system service is not installed"},
				},
				Suggestions: []poc.Suggestion{
					{Message: "run: sudo miopunch install-system-daemon"},
				},
			})
		}
		return exitWithFailure(opt, stdout, stderr, "uninstall-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to query system service: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{{Message: "retry"}},
		})
	}

	_ = svc.Stop()
	if err := svc.Uninstall(); err != nil {
		return exitWithFailure(opt, stdout, stderr, "uninstall-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to uninstall system service: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{{Message: "retry"}},
		})
	}

	_ = os.Remove(linuxStableBinaryPath)

	fmt.Fprintln(stderr, "uninstalled miopunch system daemon (state preserved)")
	fmt.Fprintf(stderr, "stable_binary=%s\n", linuxStableBinaryPath)
	return 0
}

func linuxSystemService() (service.Service, error) {
	cfg := &service.Config{
		Name:        "miopunch",
		DisplayName: "miopunch",
		Description: "miopunch LocalAPI daemon (miopunch up)",
		Executable:  linuxStableBinaryPath,
		Arguments:   []string{"up"},
	}
	prg := &noopServiceProgram{}
	return service.New(prg, cfg)
}

type noopServiceProgram struct{}

func (p *noopServiceProgram) Start(service.Service) error { return nil }
func (p *noopServiceProgram) Stop(service.Service) error  { return nil }

func installStableBinary(dest string) error {
	exe, err := os.Executable()
	if err != nil {
		return fmt.Errorf("determine current executable: %w", err)
	}
	exe, err = filepath.EvalSymlinks(exe)
	if err != nil {
		return fmt.Errorf("resolve executable symlink: %w", err)
	}

	dir := filepath.Dir(dest)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return fmt.Errorf("create dest dir: %w", err)
	}

	tmp := dest + ".tmp"
	if err := copyFile(tmp, exe, 0o755); err != nil {
		return err
	}
	return os.Rename(tmp, dest)
}

func copyFile(dest string, src string, mode os.FileMode) error {
	in, err := os.Open(src)
	if err != nil {
		return fmt.Errorf("open src: %w", err)
	}
	defer func() { _ = in.Close() }()

	out, err := os.OpenFile(dest, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, mode)
	if err != nil {
		return fmt.Errorf("create dest: %w", err)
	}
	defer func() {
		_ = out.Close()
	}()

	if _, err := io.Copy(out, in); err != nil {
		return fmt.Errorf("copy: %w", err)
	}
	return out.Close()
}

func ensureServiceInstalled(svc service.Service) error {
	_, err := svc.Status()
	if err == nil {
		return nil
	}
	if errors.Is(err, service.ErrNotInstalled) {
		return svc.Install()
	}
	return err
}

func ensureLinuxOperatorAccess() (string, error) {
	operatorUser, err := resolveLinuxOperatorUser()
	if err != nil {
		return "", err
	}

	if err := ensureLinuxGroupExists(poc.LinuxOperatorGroup); err != nil {
		return operatorUser, err
	}

	if operatorUser == "root" {
		return operatorUser, nil
	}

	inGroup, err := linuxUserInGroup(operatorUser, poc.LinuxOperatorGroup)
	if err != nil {
		return operatorUser, err
	}
	if inGroup {
		return operatorUser, nil
	}

	if err := addLinuxUserToGroup(operatorUser, poc.LinuxOperatorGroup); err != nil {
		return operatorUser, err
	}
	return operatorUser, nil
}

func resolveLinuxOperatorUser() (string, error) {
	currentUser := ""
	current, err := user.Current()
	if err == nil {
		currentUser = current.Username
	}

	return pickLinuxOperatorUser(
		os.Getenv("SUDO_USER"),
		os.Getenv("DOAS_USER"),
		os.Getenv("PKEXEC_UID"),
		func(uid string) (string, error) {
			account, err := user.LookupId(uid)
			if err != nil {
				return "", err
			}
			return strings.TrimSpace(account.Username), nil
		},
		currentUser,
	)
}

func pickLinuxOperatorUser(
	sudoUser string,
	doasUser string,
	pkexecUID string,
	lookupUsernameByID func(string) (string, error),
	currentUser string,
) (string, error) {
	if username := strings.TrimSpace(sudoUser); username != "" {
		return username, nil
	}
	if username := strings.TrimSpace(doasUser); username != "" {
		return username, nil
	}
	if uid := strings.TrimSpace(pkexecUID); uid != "" {
		username, err := lookupUsernameByID(uid)
		if err != nil {
			return "", fmt.Errorf("lookup pkexec uid %q: %w", uid, err)
		}
		if username == "" {
			return "", fmt.Errorf("lookup pkexec uid %q returned empty username", uid)
		}
		return username, nil
	}

	if username := strings.TrimSpace(currentUser); username != "" {
		return username, nil
	}
	return "", errors.New("current user has empty username")
}

func ensureLinuxGroupExists(groupName string) error {
	if _, err := user.LookupGroup(groupName); err == nil {
		return nil
	}

	return runLinuxAdminCommand(
		[]string{"groupadd", "--force", groupName},
		[]string{"addgroup", "--system", groupName},
	)
}

func linuxUserInGroup(username string, groupName string) (bool, error) {
	account, err := user.Lookup(username)
	if err != nil {
		return false, fmt.Errorf("lookup user %q: %w", username, err)
	}
	group, err := user.LookupGroup(groupName)
	if err != nil {
		return false, fmt.Errorf("lookup group %q: %w", groupName, err)
	}

	groupIDs, err := account.GroupIds()
	if err != nil {
		return false, fmt.Errorf("lookup groups for user %q: %w", username, err)
	}
	for _, groupID := range groupIDs {
		if groupID == group.Gid {
			return true, nil
		}
	}
	return false, nil
}

func addLinuxUserToGroup(username string, groupName string) error {
	return runLinuxAdminCommand(
		[]string{"usermod", "-aG", groupName, username},
		[]string{"gpasswd", "-a", username, groupName},
		[]string{"adduser", username, groupName},
	)
}

func runLinuxAdminCommand(candidates ...[]string) error {
	var missing []string

	for _, candidate := range candidates {
		if len(candidate) == 0 {
			continue
		}

		path, err := exec.LookPath(candidate[0])
		if err != nil {
			missing = append(missing, candidate[0])
			continue
		}

		cmd := exec.Command(path, candidate[1:]...)
		output, err := cmd.CombinedOutput()
		if err != nil {
			text := strings.TrimSpace(string(output))
			if text == "" {
				return fmt.Errorf("%s: %w", strings.Join(candidate, " "), err)
			}
			return fmt.Errorf("%s: %w: %s", strings.Join(candidate, " "), err, text)
		}
		return nil
	}

	if len(missing) == 0 {
		return errors.New("no linux admin command candidates provided")
	}
	return fmt.Errorf("missing required system command(s): %s", strings.Join(missing, ", "))
}
