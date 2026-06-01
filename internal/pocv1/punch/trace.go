package punch

import (
	"context"
	"fmt"
	"sort"
	"strings"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/eventctx"
	"github.com/miopunch/miopunch/internal/logutil"
)

func withAttemptTraceLogger(ctx context.Context, sid string) context.Context {
	return eventctx.WithEmitFunc(ctx, func(ev event.Event) {
		logutil.Tracef(
			"pocv1 punch event: sid=%s stage=%s kind=%s name=%s msg=%s err=%s kvs=%s",
			strings.TrimSpace(sid),
			ev.Stage,
			ev.Kind,
			strings.TrimSpace(ev.Name),
			strings.TrimSpace(ev.Msg),
			strings.TrimSpace(ev.Err),
			formatTraceEventKVs(ev.KVs),
		)
	})
}

func formatTraceEventKVs(kvs map[string]any) string {
	if len(kvs) == 0 {
		return "-"
	}
	keys := make([]string, 0, len(kvs))
	for key := range kvs {
		keys = append(keys, key)
	}
	sort.Strings(keys)

	parts := make([]string, 0, len(keys))
	for _, key := range keys {
		parts = append(parts, fmt.Sprintf("%s=%v", key, kvs[key]))
	}
	return strings.Join(parts, ",")
}
