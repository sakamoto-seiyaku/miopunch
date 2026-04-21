package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/task"
)

func runLS(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	_ = args

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	c, _, err := connectLocalAPI(ctx, opt.LocalAPIOverride)
	if err != nil {
		return exitWithError(opt, stdout, stderr, "ls", "", err)
	}

	peers, err := c.GetPeers(ctx)
	if err != nil {
		return exitWithError(opt, stdout, stderr, "ls", "", err)
	}

	if opt.Format == outputFormatJSON {
		env := poc.NewEnvelopeJSONV0()
		env.Kind = "ls"
		env.Status = "done"
		env.Stage = string(poc.StageControlPlaneReady)
		env.ReasonCode = poc.ReasonCodeOK
		env.Facts = append(env.Facts, poc.Fact{Message: fmt.Sprintf("peer_count=%d", len(peers.Peers))})
		writeEnvelopeJSON(stdout, env)
		return 0
	}

	for _, p := range peers.Peers {
		fmt.Fprintln(stdout, p.PeerID)
	}
	return 0
}

func runJoin(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	var joinArgs any
	if len(args) >= 1 && strings.TrimSpace(args[0]) != "" {
		joinArgs = task.JoinArgs{Code: args[0]}
	}
	return runTaskKind(opt, "join", joinArgs, stdout, stderr)
}

func runPing(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	var pingArgs any
	if len(args) >= 1 && strings.TrimSpace(args[0]) != "" {
		pingArgs = task.PingArgs{PeerID: args[0]}
	}
	return runTaskKind(opt, "ping", pingArgs, stdout, stderr)
}

func runShLS(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	peerID := ""
	if len(args) >= 1 {
		peerID = args[0]
	}
	target := ""
	if len(args) >= 2 {
		target = args[1]
	}
	var shArgs any
	if strings.TrimSpace(peerID) != "" {
		shArgs = task.ShLSArgs{PeerID: peerID, Target: target}
	}
	return runTaskKind(opt, "sh_ls", shArgs, stdout, stderr)
}

func runTaskKind(opt globalOptions, kind string, args any, stdout, stderr io.Writer) int {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()

	c, _, err := connectLocalAPI(ctx, opt.LocalAPIOverride)
	if err != nil {
		return exitWithError(opt, stdout, stderr, kind, "", err)
	}

	eventsBody, err := c.OpenEvents(ctx)
	if err != nil {
		return exitWithError(opt, stdout, stderr, kind, "", err)
	}
	defer func() { _ = eventsBody.Close() }()

	r := bufio.NewReader(eventsBody)
	firstData, err := readNextSSEData(r)
	if err != nil {
		return exitWithFailure(opt, stdout, stderr, kind, "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to read sse snapshot: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{{Message: "retry"}},
		})
	}
	var firstEv task.Event
	if err := json.Unmarshal([]byte(firstData), &firstEv); err != nil || strings.TrimSpace(firstEv.Kind) == "" {
		return exitWithFailure(opt, stdout, stderr, kind, "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "invalid sse snapshot event"},
			},
			Suggestions: []poc.Suggestion{{Message: "retry"}},
		})
	}
	if firstEv.Kind != "snapshot" {
		return exitWithFailure(opt, stdout, stderr, kind, "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "sse did not start with snapshot"},
				{Message: "kind=" + firstEv.Kind},
			},
			Suggestions: []poc.Suggestion{{Message: "retry"}},
		})
	}

	created, err := c.CreateTask(ctx, kind, args)
	if err != nil {
		return exitWithError(opt, stdout, stderr, kind, "", err)
	}

	if opt.Format == outputFormatHuman {
		fmt.Fprintf(stderr, "task_id=%s\n", created.ID)
	}

	if err := waitForTaskDoneEvent(r, created.ID); err != nil {
		if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
			return exitWithFailure(opt, stdout, stderr, kind, created.ID, failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeTimeout,
				ExitCode:   poc.ExitCodeTimeout,
				Facts: []poc.Fact{
					{Message: "timed out waiting for task completion"},
				},
				Suggestions: []poc.Suggestion{
					{Message: "retry"},
				},
			})
		}
		return exitWithFailure(opt, stdout, stderr, kind, created.ID, failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts: []poc.Fact{
				{Message: "failed to read sse events: " + err.Error()},
			},
			Suggestions: []poc.Suggestion{
				{Message: "retry"},
			},
		})
	}

	finalTask, err := c.GetTask(ctx, created.ID)
	if err != nil {
		return exitWithError(opt, stdout, stderr, kind, created.ID, err)
	}

	env := envelopeFromTask(finalTask)
	if opt.Format == outputFormatJSON {
		writeEnvelopeJSON(stdout, env)
		return int(finalTask.ExitCode)
	}

	if env.ExitCode != poc.ExitCodeOK {
		writeFailure(stderr, failureOutput{
			Stage:       env.Stage,
			ReasonCode:  env.ReasonCode,
			ExitCode:    env.ExitCode,
			Facts:       env.Facts,
			Suggestions: env.Suggestions,
		})
		return int(finalTask.ExitCode)
	}

	for _, fact := range env.Facts {
		msg := strings.TrimSpace(fact.Message)
		if msg == "" {
			continue
		}
		fmt.Fprintln(stdout, msg)
	}
	for _, suggestion := range env.Suggestions {
		msg := strings.TrimSpace(suggestion.Message)
		if msg == "" {
			continue
		}
		fmt.Fprintln(stderr, msg)
	}
	return int(finalTask.ExitCode)
}

func envelopeFromTask(t task.Task) poc.EnvelopeJSONV0 {
	env := poc.NewEnvelopeJSONV0()
	env.TaskID = t.ID
	env.Kind = t.Kind
	env.Status = string(t.Status)
	env.Stage = string(t.Stage)
	env.ReasonCode = t.ReasonCode
	env.ExitCode = t.ExitCode
	env.Facts = append([]poc.Fact{}, t.Facts...)
	env.Suggestions = append([]poc.Suggestion{}, t.Suggestions...)
	return env
}

func waitForTaskDoneEvent(r *bufio.Reader, taskID string) error {
	for {
		data, err := readNextSSEData(r)
		if err != nil {
			return err
		}

		var ev task.Event
		if err := json.Unmarshal([]byte(data), &ev); err != nil {
			continue
		}

		if ev.Kind != "done" {
			continue
		}
		if ev.TaskID != taskID {
			continue
		}
		return nil
	}
}

func readNextSSEData(r *bufio.Reader) (string, error) {
	var data string
	for {
		line, err := r.ReadString('\n')
		if err != nil {
			return "", err
		}
		line = strings.TrimRight(line, "\r\n")
		if line == "" {
			if data != "" {
				return data, nil
			}
			continue
		}
		if strings.HasPrefix(line, ":") {
			continue
		}
		if strings.HasPrefix(line, "data: ") {
			data = strings.TrimPrefix(line, "data: ")
		}
	}
}

func exitWithError(opt globalOptions, stdout, stderr io.Writer, kind string, taskID string, err error) int {
	var apiErr *localapi.APIError
	if errors.As(err, &apiErr) && apiErr != nil {
		return exitWithFailure(opt, stdout, stderr, kind, taskID, failureOutput{
			Stage:       apiErr.Response.Stage,
			ReasonCode:  apiErr.Response.ReasonCode,
			ExitCode:    apiErr.Response.ExitCode,
			Facts:       apiErr.Response.Facts,
			Suggestions: apiErr.Response.Suggestions,
		})
	}

	var connErr *localAPIConnectionError
	if errors.As(err, &connErr) && connErr != nil {
		return exitWithFailure(opt, stdout, stderr, kind, taskID, connErr.Failure)
	}

	return exitWithFailure(opt, stdout, stderr, kind, taskID, failureOutput{
		Stage:      "cli",
		ReasonCode: poc.ReasonCodeInternal,
		ExitCode:   poc.ExitCodeInternal,
		Facts: []poc.Fact{
			{Message: "unexpected error: " + err.Error()},
		},
		Suggestions: []poc.Suggestion{{Message: "retry"}},
	})
}

func exitWithFailure(opt globalOptions, stdout, stderr io.Writer, kind string, taskID string, f failureOutput) int {
	if opt.Format == outputFormatJSON {
		env := poc.NewEnvelopeJSONV0()
		env.TaskID = taskID
		env.Kind = kind
		env.Status = "done"
		env.Stage = f.Stage
		env.ReasonCode = f.ReasonCode
		env.ExitCode = f.ExitCode
		env.Facts = append([]poc.Fact{}, f.Facts...)
		env.Suggestions = append([]poc.Suggestion{}, f.Suggestions...)
		writeEnvelopeJSON(stdout, env)
	} else {
		writeFailure(stderr, f)
	}
	return int(f.ExitCode)
}
