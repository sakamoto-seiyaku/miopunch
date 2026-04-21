package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
	"golang.org/x/term"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/shellproto"
	"github.com/miopunch/miopunch/internal/task"
)

func runSh(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	if opt.Format != outputFormatHuman {
		return exitWithFailure(opt, stdout, stderr, "sh", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts: []poc.Fact{
				{Message: "--format json is not supported for interactive sh"},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry without --format json"},
			},
		})
	}

	fs := flag.NewFlagSet("sh", flag.ContinueOnError)
	fs.SetOutput(io.Discard)
	var session string
	fs.StringVar(&session, "s", "main", "tmux session name")
	fs.StringVar(&session, "session", "main", "tmux session name")

	_ = fs.Parse(args)
	rest := fs.Args()

	if len(rest) < 1 || strings.TrimSpace(rest[0]) == "" {
		return exitWithFailure(opt, stdout, stderr, "sh", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts: []poc.Fact{
				{Message: "missing peer_id"},
			},
			Suggestions: []poc.Suggestion{
				{Message: "use: miopunch sh <peer_id> [target] [-s session]"},
			},
		})
	}

	peerID := rest[0]
	target := ""
	if len(rest) >= 2 {
		target = rest[1]
	}

	return runShellInteractive(opt, task.ShAttachArgs{
		PeerID:  peerID,
		Target:  target,
		Session: strings.TrimSpace(session),
	}, stdout, stderr)
}

type wsWrite struct {
	msgType int
	data    []byte
}

func runShellInteractive(opt globalOptions, args task.ShAttachArgs, stdout, stderr io.Writer) int {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	apiCtx, cancelAPI := context.WithTimeout(ctx, 5*time.Second)
	defer cancelAPI()

	c, _, err := connectLocalAPI(apiCtx, opt.LocalAPIOverride)
	if err != nil {
		return exitWithError(opt, stdout, stderr, "sh", "", err)
	}

	createCtx, cancelCreate := context.WithTimeout(ctx, 10*time.Second)
	defer cancelCreate()

	created, err := c.CreateTask(createCtx, "sh_attach", args)
	if err != nil {
		return exitWithError(opt, stdout, stderr, "sh_attach", "", err)
	}
	fmt.Fprintf(stderr, "task_id=%s\n", created.ID)

	wsCtx, cancelWS := context.WithTimeout(ctx, 10*time.Second)
	defer cancelWS()

	conn, resp, err := c.DialTaskWS(wsCtx, created.ID)
	if resp != nil && resp.Body != nil {
		_ = resp.Body.Close()
	}
	if err != nil {
		return exitWithFailure(opt, stdout, stderr, "sh_attach", created.ID, failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeUnavailable,
			ExitCode:   poc.ExitCodeUnavailable,
			Facts: []poc.Fact{
				{Message: "websocket connect failed: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
	}
	defer func() { _ = conn.Close() }()

	stdinFD := int(os.Stdin.Fd())
	var restoreTerm func()
	if term.IsTerminal(stdinFD) {
		oldState, err := term.MakeRaw(stdinFD)
		if err == nil {
			restoreTerm = func() { _ = term.Restore(stdinFD, oldState) }
			defer restoreTerm()
		}
	}

	wsWriteCh := make(chan wsWrite, 64)
	var wg sync.WaitGroup

	wg.Add(1)
	go func() {
		defer wg.Done()
		defer cancel()

		for {
			select {
			case <-ctx.Done():
				return
			case msg := <-wsWriteCh:
				if err := conn.WriteMessage(msg.msgType, msg.data); err != nil {
					return
				}
			}
		}
	}()

	sendWinSize := func(cols, rows int) {
		if cols <= 0 || rows <= 0 {
			return
		}
		payload, _ := json.Marshal(shellproto.Control{
			Op:      shellproto.OpWinSize,
			WinSize: &shellproto.WinSize{Cols: cols, Rows: rows},
		})
		select {
		case wsWriteCh <- wsWrite{msgType: websocket.TextMessage, data: payload}:
		case <-ctx.Done():
		}
	}

	if cols, rows, err := term.GetSize(stdinFD); err == nil {
		sendWinSize(cols, rows)
	}
	watchResize(ctx, stdinFD, sendWinSize)

	wg.Add(1)
	go func() {
		defer wg.Done()
		defer cancel()

		buf := make([]byte, 32*1024)
		for {
			n, err := os.Stdin.Read(buf)
			if n > 0 {
				payload := append([]byte(nil), buf[:n]...)
				select {
				case wsWriteCh <- wsWrite{msgType: websocket.BinaryMessage, data: payload}:
				case <-ctx.Done():
					return
				}
			}
			if err != nil {
				return
			}
		}
	}()

	for {
		mt, payload, err := conn.ReadMessage()
		if err != nil {
			break
		}
		if mt == websocket.BinaryMessage {
			_, _ = stdout.Write(payload)
		}
	}

	cancel()
	wg.Wait()

	finalCtx, cancelFinal := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancelFinal()

	finalTask, err := waitForTaskDone(finalCtx, c, created.ID)
	if err != nil {
		return 0
	}
	if finalTask.ExitCode != poc.ExitCodeOK {
		writeFailure(stderr, failureOutput{
			Stage:       string(finalTask.Stage),
			ReasonCode:  finalTask.ReasonCode,
			ExitCode:    finalTask.ExitCode,
			Facts:       finalTask.Facts,
			Suggestions: finalTask.Suggestions,
		})
	}
	return int(finalTask.ExitCode)
}

func waitForTaskDone(ctx context.Context, c *localapi.Client, taskID string) (task.Task, error) {
	deadline := time.Now().Add(4 * time.Second)
	for {
		t, err := c.GetTask(ctx, taskID)
		if err == nil && t.Status == task.StatusDone {
			return t, nil
		}
		if time.Now().After(deadline) {
			if err != nil {
				return task.Task{}, err
			}
			return t, fmt.Errorf("task not done")
		}
		select {
		case <-ctx.Done():
			return task.Task{}, ctx.Err()
		case <-time.After(50 * time.Millisecond):
		}
	}
}
