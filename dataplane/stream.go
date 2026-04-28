package dataplane

import (
	"context"
	"fmt"
	"io"
	"net"

	"github.com/miopunch/miopunch/event"
)

// DialStream establishes the selected data plane over the already-working UDP path
// and opens a default shell logical stream.
//
// The returned ReadWriteCloser owns the peer session and MUST be closed.
func DialStream(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, em *event.Emitter) (io.ReadWriteCloser, error) {
	sess, err := DialSession(ctx, cfg, listenConn, raddr, em)
	if err != nil {
		return nil, err
	}
	stream, err := sess.OpenStream(ctx, StreamOpen{Kind: StreamKindShellV0})
	if err != nil {
		_ = sess.Close(CloseReasonTransportFatal)
		return nil, err
	}
	return &sessionOwnedStream{ReadWriteCloser: stream, session: sess}, nil
}

// ServeStream accepts / serves the selected data plane over the already-working UDP path
// and accepts a default shell logical stream.
//
// The returned ReadWriteCloser owns the peer session and MUST be closed.
func ServeStream(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, em *event.Emitter) (io.ReadWriteCloser, error) {
	sess, err := ServeSession(ctx, cfg, listenConn, raddr, em)
	if err != nil {
		return nil, err
	}
	accepted, err := sess.AcceptStream(ctx)
	if err != nil {
		_ = sess.Close(CloseReasonTransportFatal)
		return nil, err
	}
	if accepted.Open.Kind != StreamKindShellV0 {
		_ = accepted.Stream.Close()
		_ = sess.Close(CloseReasonStreamProtocolError)
		return nil, fmt.Errorf("unexpected stream kind: %q", accepted.Open.Kind)
	}
	return &sessionOwnedStream{ReadWriteCloser: accepted.Stream, session: sess}, nil
}
