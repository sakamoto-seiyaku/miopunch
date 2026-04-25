package wire

import (
	"errors"
	"io"
	"net"
	"testing"
	"time"
)

func TestDispatcher_RegisterHandlerAfterRun(t *testing.T) {
	local, remote := net.Pipe()
	t.Cleanup(func() { _ = local.Close() })
	t.Cleanup(func() { _ = remote.Close() })

	disp := NewDispatcher(local)
	disp.Run()

	handled := make(chan *PeerHello, 1)
	disp.RegisterHandler(&PeerHello{}, func(m Message) {
		hello, ok := m.(*PeerHello)
		if !ok {
			return
		}
		handled <- hello
	})

	want := &PeerHello{Role: "client", ProxyName: "proxy"}
	if err := WriteMsg(remote, want); err != nil {
		t.Fatalf("WriteMsg(PeerHello) error = %v, want nil", err)
	}

	select {
	case got := <-handled:
		if got.Role != want.Role || got.ProxyName != want.ProxyName {
			t.Fatalf("dispatcher handler got PeerHello = %#v, want %#v", got, want)
		}
	case <-time.After(2 * time.Second):
		t.Fatalf("dispatcher handler was not called for PeerHello")
	}
}

func TestDispatcher_WriteFailureClosesDoneWithError(t *testing.T) {
	wantErr := errors.New("write failed")
	rw := &blockingFailReadWriter{
		readUnblock: make(chan struct{}),
		writeErr:    wantErr,
	}
	t.Cleanup(func() { close(rw.readUnblock) })

	disp := NewDispatcher(rw)
	disp.Run()

	if err := disp.Send(&PeerHello{Role: "client"}); err != nil {
		t.Fatalf("Dispatcher.Send(PeerHello) error = %v, want nil before write failure", err)
	}

	select {
	case <-disp.Done():
	case <-time.After(2 * time.Second):
		t.Fatalf("Dispatcher.Done() was not closed after write failure")
	}

	if err := disp.Err(); !errors.Is(err, wantErr) {
		t.Fatalf("Dispatcher.Err() = %v, want %v", err, wantErr)
	}
	if err := disp.Send(&PeerHello{Role: "client"}); !errors.Is(err, wantErr) {
		t.Fatalf("Dispatcher.Send(PeerHello) after done error = %v, want %v", err, wantErr)
	}
}

type blockingFailReadWriter struct {
	readUnblock chan struct{}
	writeErr    error
}

func (rw *blockingFailReadWriter) Read(_ []byte) (int, error) {
	<-rw.readUnblock
	return 0, io.EOF
}

func (rw *blockingFailReadWriter) Write(_ []byte) (int, error) {
	return 0, rw.writeErr
}
