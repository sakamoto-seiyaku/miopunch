package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
	"golang.org/x/term"

	"github.com/miopunch/miopunch/connectivity"
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

	peerID := ""
	target := ""
	session := "main"
	p2pNetwork := "auto"

	i := 0
	for i < len(args) {
		a := args[i]
		if a == "--" {
			i++
			break
		}

		switch {
		case a == "-s" || a == "-session" || a == "--session":
			if i+1 >= len(args) {
				return exitWithFailure(opt, stdout, stderr, "sh", "", failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "missing value for --session"}},
					Suggestions: []poc.Suggestion{
						{Message: "use: miopunch sh <peer_id> [target] [-s session]"},
					},
				})
			}
			session = args[i+1]
			i += 2
			continue
		case strings.HasPrefix(a, "-s="):
			session = strings.TrimPrefix(a, "-s=")
			i++
			continue
		case strings.HasPrefix(a, "--session="):
			session = strings.TrimPrefix(a, "--session=")
			i++
			continue
		case strings.HasPrefix(a, "-session="):
			session = strings.TrimPrefix(a, "-session=")
			i++
			continue
		case a == "-u":
			p2pNetwork = "udp_only"
			i++
			continue
		case a == "-t":
			p2pNetwork = "tcp_only"
			i++
			continue
		case a == "--p2p-network":
			if i+1 >= len(args) {
				return exitWithFailure(opt, stdout, stderr, "sh", "", failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "missing value for --p2p-network"}},
					Suggestions: []poc.Suggestion{
						{Message: "use: miopunch sh <peer_id> [target] --p2p-network auto|udp_only|tcp_only"},
					},
				})
			}
			p2pNetwork = args[i+1]
			i += 2
			continue
		case strings.HasPrefix(a, "--p2p-network="):
			p2pNetwork = strings.TrimPrefix(a, "--p2p-network=")
			i++
			continue
		case strings.HasPrefix(a, "-"):
			return exitWithFailure(opt, stdout, stderr, "sh", "", failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Facts:      []poc.Fact{{Message: "unknown arg: " + a}},
				Suggestions: []poc.Suggestion{
					{Message: "use: miopunch sh <peer_id> [target] [-s session] [-u|-t|--p2p-network ...]"},
				},
			})
		default:
			if peerID == "" {
				peerID = a
			} else if target == "" {
				target = a
			}
			i++
			continue
		}
	}

	for ; i < len(args); i++ {
		if peerID == "" {
			peerID = args[i]
			continue
		}
		if target == "" {
			target = args[i]
			continue
		}
	}

	if strings.TrimSpace(peerID) == "" {
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

	network, err := connectivity.ParseP2PNetwork(p2pNetwork)
	if err != nil {
		return exitWithFailure(opt, stdout, stderr, "sh", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts:      []poc.Fact{{Message: err.Error()}},
			Suggestions: []poc.Suggestion{
				{Message: "use: miopunch sh <peer_id> [target] --p2p-network auto|udp_only|tcp_only"},
			},
		})
	}

	return runShellInteractive(opt, task.ShAttachArgs{
		PeerID:     peerID,
		Target:     target,
		Session:    strings.TrimSpace(session),
		P2PNetwork: string(network),
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

	if strings.TrimSpace(opt.ReportPath) != "" {
		reportCtx, cancelReport := context.WithTimeout(context.Background(), 5*time.Second)
		err := exportTaskReport(reportCtx, c, created.ID, opt.ReportPath, opt.Redact)
		cancelReport()
		if err != nil {
			writeFailure(stderr, failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeInternal,
				ExitCode:   poc.ExitCodeInternal,
				Facts: []poc.Fact{
					{Message: "export report: " + err.Error()},
				},
				Suggestions: []poc.Suggestion{
					{Message: "check --report path and retry"},
				},
			})
			return int(poc.ExitCodeInternal)
		}
	}

	if finalTask.ExitCode != poc.ExitCodeOK {
		facts := finalTask.Facts
		suggestions := finalTask.Suggestions
		if opt.Redact {
			facts = redactFacts(facts)
			suggestions = redactSuggestions(suggestions)
		}
		writeFailure(stderr, failureOutput{
			Stage:       string(finalTask.Stage),
			ReasonCode:  finalTask.ReasonCode,
			ExitCode:    finalTask.ExitCode,
			Facts:       facts,
			Suggestions: suggestions,
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
