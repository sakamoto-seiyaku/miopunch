package atomicfile

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// WriteFile writes data to path atomically:
// - write a temp file in the same directory
// - fsync the temp file
// - rename into place (best-effort replace-existing)
// - best-effort fsync the parent directory
func WriteFile(path string, data []byte, perm os.FileMode) error {
	path = strings.TrimSpace(path)
	if path == "" {
		return errors.New("empty path")
	}

	dir := filepath.Dir(path)
	base := filepath.Base(path)

	tmp, err := os.CreateTemp(dir, base+".tmp-*")
	if err != nil {
		return fmt.Errorf("create temp file: %w", err)
	}

	tmpName := tmp.Name()
	cleanup := func() {
		_ = tmp.Close()
		_ = os.Remove(tmpName)
	}

	if _, err := tmp.Write(data); err != nil {
		cleanup()
		return fmt.Errorf("write temp file: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		cleanup()
		return fmt.Errorf("fsync temp file: %w", err)
	}
	if err := tmp.Close(); err != nil {
		cleanup()
		return fmt.Errorf("close temp file: %w", err)
	}

	if perm != 0 {
		if err := os.Chmod(tmpName, perm); err != nil {
			_ = os.Remove(tmpName)
			return fmt.Errorf("chmod temp file: %w", err)
		}
	}

	if err := os.Rename(tmpName, path); err != nil {
		if _, statErr := os.Stat(path); statErr != nil {
			_ = os.Remove(tmpName)
			return fmt.Errorf("rename temp file: %w", err)
		}

		if removeErr := os.Remove(path); removeErr != nil && !errors.Is(removeErr, os.ErrNotExist) {
			_ = os.Remove(tmpName)
			return fmt.Errorf("remove existing file: %w", removeErr)
		}

		if err2 := os.Rename(tmpName, path); err2 == nil {
			return fsyncDirBestEffort(dir)
		} else {
			_ = os.Remove(tmpName)
			return fmt.Errorf("rename temp file: %w", err2)
		}
	}

	return fsyncDirBestEffort(dir)
}

func fsyncDirBestEffort(dir string) error {
	d, err := os.Open(dir)
	if err != nil {
		return nil
	}
	_ = d.Sync()
	_ = d.Close()
	return nil
}
