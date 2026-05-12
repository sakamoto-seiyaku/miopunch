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

	return readLocalAPIEvents(ctx, r, func(data []byte) (task.Event, error) {
		var ev task.Event
		if err := json.Unmarshal(data, &ev); err != nil {
			return task.Event{}, fmt.Errorf("decode task event: %w", err)
		}
		return ev, nil
	}, onEvent)
}

func ReadLocalAPIDesktopStateEvents(ctx context.Context, r io.Reader, onEvent func(task.DesktopStateEvent) error) error {
	if onEvent == nil {
		return fmt.Errorf("onEvent is nil")
	}

	return readLocalAPIEvents(ctx, r, func(data []byte) (task.DesktopStateEvent, error) {
		var ev task.DesktopStateEvent
		if err := json.Unmarshal(data, &ev); err != nil {
			return task.DesktopStateEvent{}, fmt.Errorf("decode desktop state event: %w", err)
		}
		return ev, nil
	}, onEvent)
}

func readLocalAPIEvents[T any](ctx context.Context, r io.Reader, decode func([]byte) (T, error), onEvent func(T) error) error {
	if decode == nil {
		return fmt.Errorf("decode is nil")
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

		ev, err := decode([]byte(data))
		if err != nil {
			return err
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
