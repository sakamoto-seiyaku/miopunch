package dataplane

import (
	"context"
	"errors"
	"io"
	"net"
	"os"
	"time"
)

var (
	errStreamOpenReadDeadlineUnsupported  = errors.New("stream open read requires read deadline support")
	errStreamOpenWriteDeadlineUnsupported = errors.New("stream open write requires write deadline support")
)

type readDeadlineReader interface {
	io.Reader
	SetReadDeadline(time.Time) error
}

type writeDeadlineWriter interface {
	io.Writer
	SetWriteDeadline(time.Time) error
}

type deadlineSetter func(time.Time) error

func writeStreamOpenWithContext(ctx context.Context, w io.Writer, open StreamOpen) error {
	writer, ok := w.(writeDeadlineWriter)
	if !ok {
		return errStreamOpenWriteDeadlineUnsupported
	}

	data, err := marshalStreamOpen(open)
	if err != nil {
		return err
	}

	return withContextDeadline(ctx, writer.SetWriteDeadline, func() error {
		return writeFrame(writer, data)
	})
}

func readStreamOpenWithContext(ctx context.Context, r io.Reader) (StreamOpen, error) {
	reader, ok := r.(readDeadlineReader)
	if !ok {
		return StreamOpen{}, errStreamOpenReadDeadlineUnsupported
	}

	var data []byte
	err := withContextDeadline(ctx, reader.SetReadDeadline, func() error {
		frame, err := readFrame(reader, maxStreamOpenFrame)
		if err != nil {
			return err
		}
		data = frame
		return nil
	})
	if err != nil {
		return StreamOpen{}, err
	}

	return unmarshalStreamOpen(data)
}

func withContextDeadline(ctx context.Context, setDeadline deadlineSetter, fn func() error) error {
	if err := ctx.Err(); err != nil {
		return err
	}

	deadline, hasDeadline := ctx.Deadline()
	if hasDeadline {
		if err := setDeadline(deadline); err != nil {
			return err
		}
	}

	stopAfterFunc := func() bool { return true }
	var afterFuncDone <-chan struct{}
	if ctx.Done() != nil {
		done := make(chan struct{})
		afterFuncDone = done
		stopAfterFunc = context.AfterFunc(ctx, func() {
			defer close(done)
			_ = setDeadline(time.Now())
		})
	}

	err := fn()
	if !stopAfterFunc() && afterFuncDone != nil {
		<-afterFuncDone
	}

	clearErr := setDeadline(time.Time{})
	if err == nil {
		return clearErr
	}

	if ctxErr := ctx.Err(); ctxErr != nil && isContextDeadlineError(err) {
		return ctxErr
	}
	if hasDeadline && isContextDeadlineError(err) && !time.Now().Before(deadline) {
		return context.DeadlineExceeded
	}

	return err
}

func isContextDeadlineError(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, os.ErrDeadlineExceeded) {
		return true
	}

	var netErr net.Error
	return errors.As(err, &netErr) && netErr.Timeout()
}
