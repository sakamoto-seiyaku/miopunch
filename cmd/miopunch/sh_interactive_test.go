package main

import (
	"bytes"
	"context"
	"errors"
	"io"
	"testing"
	"time"

	"golang.org/x/term"

	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
)

func TestRunShellInteractive_RemoteCloseDoesNotWaitForIdleStdin(t *testing.T) {
	stdinReader, stdinWriter := io.Pipe()
	t.Cleanup(func() { _ = stdinReader.Close() })
	t.Cleanup(func() { _ = stdinWriter.Close() })

	client := &fakeShellClient{
		result: pocruntime.ActionResult{
			ExitCode:       poc.ExitCodeOK,
			ReasonCode:     poc.ReasonCodeOK,
			ShellSessionID: "shell-1",
		},
		stream: fakeShellStream{readErr: io.EOF},
	}
	deps := shellInteractiveDeps{
		connect: func(context.Context, string) (shellClient, error) {
			return client, nil
		},
		stdin:       stdinReader,
		stdinFD:     func() int { return -1 },
		isTerminal:  func(int) bool { return false },
		makeRaw:     func(int) (*term.State, error) { return nil, errors.New("not a terminal") },
		restoreTerm: func(int, *term.State) error { return nil },
		getSize: func(int) (int, int, error) {
			return 0, 0, errors.New("not a terminal")
		},
		watchResize: func(context.Context, int, func(int, int)) {},
	}

	done := make(chan int, 1)
	go func() {
		var stdout bytes.Buffer
		var stderr bytes.Buffer
		done <- runShellInteractiveWithDeps(globalOptions{}, pocruntime.ShellArgs{PeerID: "peer"}, &stdout, &stderr, deps)
	}()

	select {
	case gotExitCode := <-done:
		if gotExitCode != int(poc.ExitCodeOK) {
			t.Fatalf("runShellInteractiveWithDeps() exitCode = %d, want %d", gotExitCode, poc.ExitCodeOK)
		}
	case <-time.After(2 * time.Second):
		t.Fatalf("runShellInteractiveWithDeps() did not return after remote close while stdin was idle")
	}
}

type fakeShellClient struct {
	result pocruntime.ActionResult
	stream io.ReadWriteCloser
}

func (c *fakeShellClient) Action(context.Context, string, any) (pocruntime.ActionResult, error) {
	return c.result, nil
}

func (c *fakeShellClient) DialShell(context.Context, string) (io.ReadWriteCloser, error) {
	return c.stream, nil
}

type fakeShellStream struct {
	readErr error
}

func (s fakeShellStream) Read([]byte) (int, error) {
	return 0, s.readErr
}

func (s fakeShellStream) Write(p []byte) (int, error) {
	return len(p), nil
}

func (s fakeShellStream) Close() error {
	return nil
}
