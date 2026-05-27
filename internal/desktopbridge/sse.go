package desktopbridge

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"strings"

	"github.com/miopunch/miopunch/internal/localapi"
)

// ReadLocalAPITaskEvents consumes a newline-delimited LocalAPI runtime event stream.
func ReadLocalAPITaskEvents(ctx context.Context, r io.Reader, onEvent func(localapi.Event) error) error {
	if onEvent == nil {
		return fmt.Errorf("onEvent is nil")
	}

	return readLocalAPIEvents(ctx, r, func(data []byte) (localapi.Event, error) {
		var ev localapi.Event
		if err := json.Unmarshal(data, &ev); err != nil {
			return localapi.Event{}, fmt.Errorf("decode task event: %w", err)
		}
		return ev, nil
	}, onEvent)
}

func ReadLocalAPIDesktopStateEvents(ctx context.Context, r io.Reader, onEvent func(localapi.Event) error) error {
	if onEvent == nil {
		return fmt.Errorf("onEvent is nil")
	}

	return readLocalAPIEvents(ctx, r, func(data []byte) (localapi.Event, error) {
		var ev localapi.Event
		if err := json.Unmarshal(data, &ev); err != nil {
			return localapi.Event{}, fmt.Errorf("decode desktop state event: %w", err)
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

	for sc.Scan() {
		if err := ctx.Err(); err != nil {
			return err
		}

		line := strings.TrimRight(sc.Text(), "\r")
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		ev, err := decode([]byte(line))
		if err != nil {
			return err
		}
		if err := onEvent(ev); err != nil {
			return err
		}
	}
	if err := sc.Err(); err != nil {
		return err
	}
	return nil
}
