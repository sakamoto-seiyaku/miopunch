package task

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/miopunch/miopunch/internal/atomicfile"
	"github.com/miopunch/miopunch/internal/pocstate"
)

const defaultReportsKeep = 20

var (
	reportsLocksMu sync.Mutex
	reportsLocks   = make(map[string]*sync.Mutex)
)

func lockForReportsDir(dir string) *sync.Mutex {
	reportsLocksMu.Lock()
	defer reportsLocksMu.Unlock()

	if mu, ok := reportsLocks[dir]; ok {
		return mu
	}
	mu := &sync.Mutex{}
	reportsLocks[dir] = mu
	return mu
}

func reportPath(stateDir string, taskID string) string {
	return filepath.Join(stateDir, "reports", taskID+".md")
}

func (m *Manager) persistReport(taskID string, report string) error {
	if m == nil {
		return errors.New("nil manager")
	}
	taskID = strings.TrimSpace(taskID)
	if taskID == "" {
		return errors.New("empty task_id")
	}

	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		return err
	}
	reportsDir := filepath.Join(stateDir, "reports")

	mu := lockForReportsDir(reportsDir)
	mu.Lock()
	defer mu.Unlock()

	if err := os.MkdirAll(reportsDir, 0o700); err != nil {
		return fmt.Errorf("mkdir reports dir: %w", err)
	}

	path := reportPath(stateDir, taskID)
	if err := atomicfile.WriteFile(path, []byte(report), 0o600); err != nil {
		return fmt.Errorf("write report: %w", err)
	}
	if err := pruneReports(reportsDir, defaultReportsKeep); err != nil {
		return fmt.Errorf("prune reports: %w", err)
	}
	return nil
}

func (m *Manager) LoadPersistedReport(taskID string) (string, bool, error) {
	if m == nil {
		return "", false, errors.New("nil manager")
	}
	taskID = strings.TrimSpace(taskID)
	if taskID == "" {
		return "", false, errors.New("empty task_id")
	}

	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		return "", false, err
	}
	path := reportPath(stateDir, taskID)

	data, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return "", false, nil
		}
		return "", false, err
	}
	return string(data), true, nil
}

func pruneReports(reportsDir string, keep int) error {
	if keep <= 0 {
		return nil
	}

	entries, err := os.ReadDir(reportsDir)
	if err != nil {
		return err
	}

	type reportInfo struct {
		name string
		path string
		mod  time.Time
	}

	reports := make([]reportInfo, 0, len(entries))
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		if filepath.Ext(e.Name()) != ".md" {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		reports = append(reports, reportInfo{
			name: e.Name(),
			path: filepath.Join(reportsDir, e.Name()),
			mod:  info.ModTime(),
		})
	}

	sort.Slice(reports, func(i, j int) bool {
		if reports[i].mod.Equal(reports[j].mod) {
			return reports[i].name > reports[j].name
		}
		return reports[i].mod.After(reports[j].mod)
	})

	for i := keep; i < len(reports); i++ {
		_ = os.Remove(reports[i].path)
	}
	return nil
}
