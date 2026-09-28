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

func TestRunShRejectsConflictingPathFlags(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name string
		args []string
	}{
		{name: "network conflict", args: []string{"peer-a", "-u", "-t"}},
		{name: "family conflict", args: []string{"peer-a", "-4", "-6"}},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			var stdout bytes.Buffer
			var stderr bytes.Buffer
			gotExitCode := runSh(globalOptions{}, tt.args, &stdout, &stderr)
			if gotExitCode != int(poc.ExitCodeBadRequest) {
				t.Fatalf("runSh(%v) exitCode = %d, want %d", tt.args, gotExitCode, poc.ExitCodeBadRequest)
			}
		})
	}
}

func TestRunShellInteractiveWithDepsPassesPathPolicy(t *testing.T) {
	t.Parallel()

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
		stdin:       bytes.NewReader(nil),
		stdinFD:     func() int { return -1 },
		isTerminal:  func(int) bool { return false },
		makeRaw:     func(int) (*term.State, error) { return nil, errors.New("not a terminal") },
		restoreTerm: func(int, *term.State) error { return nil },
		getSize: func(int) (int, int, error) {
			return 0, 0, errors.New("not a terminal")
		},
		watchResize: func(context.Context, int, func(int, int)) {},
	}

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	args := pocruntime.ShellArgs{
		PeerID:      "peer-a",
		Target:      "wsl:Debian",
		Session:     "main",
		P2PNetwork:  "udp_only",
		P2PIPFamily: "v4",
	}
	gotExitCode := runShellInteractiveWithDeps(globalOptions{}, args, &stdout, &stderr, deps)
	if gotExitCode != int(poc.ExitCodeOK) {
		t.Fatalf("runShellInteractiveWithDeps() exitCode = %d, want %d", gotExitCode, poc.ExitCodeOK)
	}
	gotArgs, ok := client.actionArgs.(pocruntime.ShellArgs)
	if !ok {
		t.Fatalf("fakeShellClient.actionArgs = %T, want ShellArgs", client.actionArgs)
	}
	if gotArgs.P2PNetwork != args.P2PNetwork || gotArgs.P2PIPFamily != args.P2PIPFamily {
		t.Fatalf("ShellArgs path policy = (%q, %q), want (%q, %q)", gotArgs.P2PNetwork, gotArgs.P2PIPFamily, args.P2PNetwork, args.P2PIPFamily)
	}
}

type fakeShellClient struct {
	result     pocruntime.ActionResult
	stream     io.ReadWriteCloser
	actionName string
	actionArgs any
}

func (c *fakeShellClient) Action(_ context.Context, action string, args any) (pocruntime.ActionResult, error) {
	c.actionName = action
	c.actionArgs = args
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
