package eventctx

import (
	"context"

	"github.com/miopunch/miopunch/event"
)

type emitKey struct{}

// EmitFunc is a callback used by Emit to forward attempt lifecycle events.
type EmitFunc func(event.Event)

// WithEmitFunc returns a context that stores emit as the event sink used by Emit.
func WithEmitFunc(ctx context.Context, emit EmitFunc) context.Context {
	if ctx == nil {
		ctx = context.Background()
	}
	if emit == nil {
		return ctx
	}
	return context.WithValue(ctx, emitKey{}, emit)
}

// Emit forwards ev to the EmitFunc stored in ctx, if any.
func Emit(ctx context.Context, ev event.Event) {
	if ctx == nil {
		return
	}
	emit, _ := ctx.Value(emitKey{}).(EmitFunc)
	if emit == nil {
		return
	}
	emit(ev)
}
