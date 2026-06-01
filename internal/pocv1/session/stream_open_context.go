// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

package session

import (
	"context"
	"errors"
	"io"
	"net"
	"os"
	"time"

	"github.com/miopunch/miopunch/dataplane"
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

	return withContextDeadline(ctx, writer.SetWriteDeadline, func() error {
		return dataplane.WriteStreamOpen(writer, open)
	})
}

func readStreamOpenWithContext(ctx context.Context, r io.Reader) (StreamOpen, error) {
	reader, ok := r.(readDeadlineReader)
	if !ok {
		return StreamOpen{}, errStreamOpenReadDeadlineUnsupported
	}

	var open StreamOpen
	err := withContextDeadline(ctx, reader.SetReadDeadline, func() error {
		acceptedOpen, err := dataplane.ReadStreamOpen(reader)
		if err != nil {
			return err
		}
		open = acceptedOpen
		return nil
	})
	if err != nil {
		return StreamOpen{}, err
	}

	return open, nil
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
