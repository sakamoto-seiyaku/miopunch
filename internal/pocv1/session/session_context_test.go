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
	"bytes"
	"context"
	"errors"
	"io"
	"testing"
	"time"

	"github.com/miopunch/miopunch/dataplane"
)

func TestAcceptStreamContextCancellationKeepsSessionUsable(t *testing.T) {
	t.Parallel()

	ctx, clientSess, serverSess := mustNewPeerSessions(t)
	rawStream, err := clientSess.sess.OpenStream()
	if err != nil {
		t.Fatalf("client yamux OpenStream() error = %v, want nil", err)
	}

	shortCtx, shortCancel := context.WithTimeout(ctx, 150*time.Millisecond)
	t.Cleanup(shortCancel)

	_, err = serverSess.AcceptStream(shortCtx)
	if !errors.Is(err, context.DeadlineExceeded) {
		_ = rawStream.Close()
		t.Fatalf("server AcceptStream() error = %v, want %v", err, context.DeadlineExceeded)
	}
	if err := rawStream.Close(); err != nil {
		t.Fatalf("rawStream.Close() error = %v, want nil", err)
	}
	if !clientSess.Healthy() {
		t.Fatalf("clientSess.Healthy() = false, want true after context cancellation")
	}
	if !serverSess.Healthy() {
		t.Fatalf("serverSess.Healthy() = false, want true after context cancellation")
	}

	acceptedCh := make(chan *dataplane.AcceptedStream, 1)
	errCh := make(chan error, 1)
	go func() {
		accepted, err := serverSess.AcceptStream(ctx)
		if err != nil {
			errCh <- err
			return
		}
		acceptedCh <- accepted
	}()

	stream, err := clientSess.OpenStream(ctx, StreamOpen{
		Kind: dataplane.StreamKindShellV0,
		Metadata: map[string]string{
			"op":    "ping",
			"trace": "retry",
		},
	})
	if err != nil {
		t.Fatalf("client OpenStream() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = stream.Close() })

	var accepted *dataplane.AcceptedStream
	select {
	case accepted = <-acceptedCh:
	case err := <-errCh:
		t.Fatalf("server AcceptStream() error = %v, want nil", err)
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for accepted stream")
	}
	if accepted == nil {
		t.Fatalf("accepted stream = nil, want non-nil")
	}
	t.Cleanup(func() { _ = accepted.Stream.Close() })
	if accepted.Open.Kind != dataplane.StreamKindShellV0 {
		t.Fatalf("accepted stream kind = %q, want %q", accepted.Open.Kind, dataplane.StreamKindShellV0)
	}
	if accepted.Open.Metadata["trace"] != "retry" {
		t.Fatalf("accepted stream trace = %q, want %q", accepted.Open.Metadata["trace"], "retry")
	}

	payload := []byte("hello-after-timeout")
	if _, err := stream.Write(payload); err != nil {
		t.Fatalf("stream.Write() error = %v, want nil", err)
	}
	buf := make([]byte, len(payload))
	if _, err := io.ReadFull(accepted.Stream, buf); err != nil {
		t.Fatalf("io.ReadFull(accepted.Stream) error = %v, want nil", err)
	}
	if !bytes.Equal(buf, payload) {
		t.Fatalf("accepted stream payload = %q, want %q", string(buf), string(payload))
	}
}

func mustNewPeerSessions(t *testing.T) (context.Context, *peerSession, *peerSession) {
	t.Helper()

	fx := mustSessionFixture(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)

	serverCh := make(chan sessionResult, 1)
	go func() {
		sess, err := Accept(ctx, fx.responderConfig(), fx.responderPath())
		serverCh <- sessionResult{session: sess, err: err}
	}()

	clientPeer, err := Dial(ctx, fx.dialerConfig(), fx.dialerPath())
	if err != nil {
		t.Fatalf("Dial() error = %v, want nil", err)
	}
	clientSess, ok := clientPeer.(*peerSession)
	if !ok {
		t.Fatalf("Dial() type = %T, want *peerSession", clientPeer)
	}
	t.Cleanup(func() { _ = clientSess.Close(dataplane.CloseReasonDaemonShutdown) })

	serverRes := <-serverCh
	if serverRes.err != nil {
		t.Fatalf("Accept() error = %v, want nil", serverRes.err)
	}
	serverSess, ok := serverRes.session.(*peerSession)
	if !ok {
		t.Fatalf("Accept() type = %T, want *peerSession", serverRes.session)
	}
	t.Cleanup(func() { _ = serverSess.Close(dataplane.CloseReasonDaemonShutdown) })

	return ctx, clientSess, serverSess
}
