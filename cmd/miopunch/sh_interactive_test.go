package main

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net/http"
	"testing"
	"time"

	"golang.org/x/term"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/task"
)

func TestRunShellInteractive_RemoteCloseDoesNotWaitForIdleStdin(t *testing.T) {
	stdinReader, stdinWriter := io.Pipe()
	t.Cleanup(func() { _ = stdinReader.Close() })
	t.Cleanup(func() { _ = stdinWriter.Close() })

	client := &fakeShellTaskClient{
		task: task.Task{
			ID:       "task-1",
			Kind:     "sh_attach",
			Status:   task.StatusDone,
			ExitCode: poc.ExitCodeOK,
		},
		ws: fakeShellWSConn{readErr: io.EOF},
	}
	deps := shellInteractiveDeps{
		connect: func(context.Context, string) (shellTaskClient, error) {
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
		done <- runShellInteractiveWithDeps(globalOptions{}, task.ShAttachArgs{PeerID: "peer"}, &stdout, &stderr, deps)
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

type fakeShellTaskClient struct {
	task task.Task
	ws   shellWSConn
}

func (c *fakeShellTaskClient) CreateTask(context.Context, string, any) (task.Task, error) {
	return c.task, nil
}

func (c *fakeShellTaskClient) DialTaskWS(context.Context, string) (shellWSConn, *http.Response, error) {
	return c.ws, nil, nil
}

func (c *fakeShellTaskClient) GetTask(context.Context, string) (task.Task, error) {
	return c.task, nil
}

func (c *fakeShellTaskClient) GetTaskReport(context.Context, string) (string, error) {
	return "", nil
}

type fakeShellWSConn struct {
	readErr error
}

func (c fakeShellWSConn) ReadMessage() (int, []byte, error) {
	return 0, nil, c.readErr
}

func (c fakeShellWSConn) WriteMessage(int, []byte) error {
	return nil
}

func (c fakeShellWSConn) Close() error {
	return nil
}
