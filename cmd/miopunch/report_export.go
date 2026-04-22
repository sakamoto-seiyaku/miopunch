package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/miopunch/miopunch/internal/atomicfile"
	"github.com/miopunch/miopunch/internal/localapi"
)

func exportTaskReport(ctx context.Context, c *localapi.Client, taskID string, path string, redact bool) error {
	path = strings.TrimSpace(path)
	if path == "" {
		return nil
	}
	if c == nil {
		return errors.New("nil localapi client")
	}

	report, err := c.GetTaskReport(ctx, taskID)
	if err != nil {
		return err
	}
	if redact {
		report = redactString(report)
	}

	dir := filepath.Dir(path)
	if dir != "." && dir != "" {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return fmt.Errorf("mkdir report dir: %w", err)
		}
	}
	if err := atomicfile.WriteFile(path, []byte(report), 0o600); err != nil {
		return fmt.Errorf("write report: %w", err)
	}
	return nil
}
