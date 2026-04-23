package desktopbridge

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"strings"

	"github.com/miopunch/miopunch/internal/task"
)

// ReadLocalAPITaskEvents consumes an SSE stream where each event is streamed as:
//
//	data: <json>
//
// The LocalAPI uses this format for /api/v0/events and /api/v0/tasks/<id>/events.
func ReadLocalAPITaskEvents(ctx context.Context, r io.Reader, onEvent func(task.Event) error) error {
	if onEvent == nil {
		return fmt.Errorf("onEvent is nil")
	}

	sc := bufio.NewScanner(r)
	sc.Buffer(make([]byte, 0, 64*1024), 1024*1024)

	var dataLines []string

	flush := func() error {
		if len(dataLines) == 0 {
			return nil
		}

		data := strings.TrimSpace(strings.Join(dataLines, "\n"))
		dataLines = dataLines[:0]
		if data == "" {
			return nil
		}

		var ev task.Event
		if err := json.Unmarshal([]byte(data), &ev); err != nil {
			return fmt.Errorf("decode task event: %w", err)
		}
		return onEvent(ev)
	}

	for sc.Scan() {
		if err := ctx.Err(); err != nil {
			return err
		}

		line := strings.TrimRight(sc.Text(), "\r")
		if line == "" {
			if err := flush(); err != nil {
				return err
			}
			continue
		}

		if strings.HasPrefix(line, ":") {
			continue
		}

		const prefix = "data:"
		if strings.HasPrefix(line, prefix) {
			dataLines = append(dataLines, strings.TrimSpace(strings.TrimPrefix(line, prefix)))
		}
	}
	if err := sc.Err(); err != nil {
		return err
	}
	return flush()
}
