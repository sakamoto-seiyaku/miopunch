//go:build windows

package main

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/kardianos/service"

	"github.com/miopunch/miopunch/internal/poc"
)

func runInstallSystemDaemon(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	_ = args
	_ = stdout

	operatorSID, err := poc.CurrentOperatorSID()
	if err != nil {
		return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to determine operator SID: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{{Message: "retry"}},
		})
	}

	stablePath, err := windowsStableBinaryPath()
	if err != nil {
		return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to determine stable binary path: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{{Message: "retry"}},
		})
	}

	if err := installStableBinary(stablePath); err != nil {
		return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to install stable binary"},
				{Message: "path=" + stablePath},
				{Message: "error=" + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry as administrator"},
			},
		})
	}

	svc, err := windowsSystemService(stablePath, operatorSID)
	if err != nil {
		return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to create service handle: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry as administrator"},
			},
		})
	}

	if err := ensureServiceInstalled(svc); err != nil {
		if isPermissionError(err) {
			return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeForbidden,
				ExitCode:   poc.ExitCodeForbidden,
				Facts: []poc.Fact{
					{Message: "permission denied installing service"},
				},
				Suggestions: []poc.Suggestion{
					{Message: "retry from an elevated Administrator prompt"},
				},
			})
		}
		return exitWithFailure(opt, stdout, stderr, "install-system-daemon", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to install service: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{{Message: "retry"}},
		})
	}

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
	fmt.Fprintf(stderr, "stable_binary=%s\n", stablePath)
	fmt.Fprintf(stderr, "operator_sid=%s\n", operatorSID)
	return 0
}

func runUninstallSystemDaemon(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	_ = args
	_ = stdout

	operatorSID, err := poc.CurrentOperatorSID()
	if err != nil {
		operatorSID = ""
	}
	stablePath, stableErr := windowsStableBinaryPath()
	if stableErr != nil {
		stablePath = ""
	}

	svc, err := windowsSystemService(stablePath, operatorSID)
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
					{Message: "run: miopunch install-system-daemon"},
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
		if isPermissionError(err) {
			return exitWithFailure(opt, stdout, stderr, "uninstall-system-daemon", "", failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeForbidden,
				ExitCode:   poc.ExitCodeForbidden,
				Facts: []poc.Fact{
					{Message: "permission denied uninstalling service"},
				},
				Suggestions: []poc.Suggestion{
					{Message: "retry from an elevated Administrator prompt"},
				},
			})
		}
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

	if stablePath != "" {
		_ = os.Remove(stablePath)
	}

	fmt.Fprintln(stderr, "uninstalled miopunch system daemon (state preserved)")
	if stablePath != "" {
		fmt.Fprintf(stderr, "stable_binary=%s\n", stablePath)
	}
	return 0
}

func windowsStableBinaryPath() (string, error) {
	root := strings.TrimSpace(os.Getenv("ProgramFiles"))
	if root == "" {
		return "", errors.New("ProgramFiles is not set")
	}
	return filepath.Join(root, "miopunch", "miopunch.exe"), nil
}

func windowsSystemService(stablePath string, operatorSID string) (service.Service, error) {
	args := []string{"up"}
	if strings.TrimSpace(operatorSID) != "" {
		args = append(args, "--operator_sid", strings.TrimSpace(operatorSID))
	}
	cfg := &service.Config{
		Name:        "miopunch",
		DisplayName: "miopunch",
		Description: "miopunch LocalAPI daemon (miopunch up)",
		Executable:  stablePath,
		Arguments:   args,
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

	if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
		return fmt.Errorf("create dest dir: %w", err)
	}

	tmp := dest + ".tmp"
	if err := copyFile(tmp, exe, 0o755); err != nil {
		return err
	}
	if err := os.Remove(dest); err != nil && !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("remove old stable binary: %w", err)
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
	defer func() { _ = out.Close() }()

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
