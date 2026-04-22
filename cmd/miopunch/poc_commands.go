package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strconv"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
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

func runInvite(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	inviteArgs := task.InviteArgs{}

	i := 0
	for i < len(args) {
		a := args[i]
		if a == "--" {
			i++
			break
		}
		if !strings.HasPrefix(a, "-") {
			break
		}

		switch {
		case a == "--mode":
			if i+1 >= len(args) {
				return exitWithFailure(opt, stdout, stderr, "invite", "", failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "missing value for --mode"}},
					Suggestions: []poc.Suggestion{
						{Message: "use: miopunch invite --mode approve|auto"},
					},
				})
			}
			i++
			inviteArgs.Mode = strings.TrimSpace(args[i])
			i++
		case strings.HasPrefix(a, "--mode="):
			inviteArgs.Mode = strings.TrimSpace(strings.TrimPrefix(a, "--mode="))
			i++
		case a == "--uses":
			if i+1 >= len(args) {
				return exitWithFailure(opt, stdout, stderr, "invite", "", failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "missing value for --uses"}},
					Suggestions: []poc.Suggestion{
						{Message: "use: miopunch invite --uses 1"},
					},
				})
			}
			i++
			n, err := strconv.Atoi(strings.TrimSpace(args[i]))
			if err != nil || n <= 0 {
				return exitWithFailure(opt, stdout, stderr, "invite", "", failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "invalid --uses"}},
					Suggestions: []poc.Suggestion{
						{Message: "use: miopunch invite --uses 1"},
					},
				})
			}
			inviteArgs.MaxUses = n
			i++
		case strings.HasPrefix(a, "--uses="):
			n, err := strconv.Atoi(strings.TrimSpace(strings.TrimPrefix(a, "--uses=")))
			if err != nil || n <= 0 {
				return exitWithFailure(opt, stdout, stderr, "invite", "", failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "invalid --uses"}},
					Suggestions: []poc.Suggestion{
						{Message: "use: miopunch invite --uses 1"},
					},
				})
			}
			inviteArgs.MaxUses = n
			i++
		case a == "--expires":
			if i+1 >= len(args) {
				return exitWithFailure(opt, stdout, stderr, "invite", "", failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "missing value for --expires"}},
					Suggestions: []poc.Suggestion{
						{Message: "use: miopunch invite --expires 15m"},
					},
				})
			}
			i++
			inviteArgs.Expires = strings.TrimSpace(args[i])
			i++
		case strings.HasPrefix(a, "--expires="):
			inviteArgs.Expires = strings.TrimSpace(strings.TrimPrefix(a, "--expires="))
			i++
		default:
			return exitWithFailure(opt, stdout, stderr, "invite", "", failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Facts:      []poc.Fact{{Message: "unknown flag: " + a}},
				Suggestions: []poc.Suggestion{
					{Message: "run: miopunch --help"},
				},
			})
		}
	}

	if len(args[i:]) != 0 {
		return exitWithFailure(opt, stdout, stderr, "invite", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts:      []poc.Fact{{Message: "unexpected extra args"}},
			Suggestions: []poc.Suggestion{
				{Message: "use: miopunch invite [--mode ...] [--uses ...] [--expires ...]"},
			},
		})
	}

	argsAny := any(inviteArgs)
	if strings.TrimSpace(inviteArgs.Mode) == "" && inviteArgs.MaxUses == 0 && strings.TrimSpace(inviteArgs.Expires) == "" {
		argsAny = nil
	}
	return runTaskKind(opt, "invite", argsAny, stdout, stderr)
}

func runApprove(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	if len(args) < 1 || strings.TrimSpace(args[0]) == "" {
		return exitWithFailure(opt, stdout, stderr, "approve", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts: []poc.Fact{
				{Message: "missing invite code"},
			},
			Suggestions: []poc.Suggestion{
				{Message: "use: miopunch approve <invite_code-or-url>"},
			},
		})
	}
	return runTaskKind(opt, "approve", task.ApproveArgs{Code: args[0]}, stdout, stderr)
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

func runRevoke(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	if len(args) < 1 || strings.TrimSpace(args[0]) == "" {
		return exitWithFailure(opt, stdout, stderr, "revoke", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts:      []poc.Fact{{Message: "missing peer_id"}},
			Suggestions: []poc.Suggestion{
				{Message: "use: miopunch revoke <peer_id> --dangerous"},
			},
		})
	}

	peerID := args[0]
	dangerous := false
	for _, a := range args[1:] {
		switch {
		case a == "--dangerous":
			dangerous = true
		case a == "--dangerous=true":
			dangerous = true
		default:
			return exitWithFailure(opt, stdout, stderr, "revoke", "", failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Facts:      []poc.Fact{{Message: "unknown arg: " + a}},
				Suggestions: []poc.Suggestion{
					{Message: "use: miopunch revoke <peer_id> --dangerous"},
				},
			})
		}
	}
	if !dangerous {
		return exitWithFailure(opt, stdout, stderr, "revoke", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts:      []poc.Fact{{Message: "missing --dangerous (revoke is irreversible in POC v0)"}},
			Suggestions: []poc.Suggestion{
				{Message: "re-run with: miopunch revoke <peer_id> --dangerous"},
			},
		})
	}

	return runTaskKind(opt, "revoke_member", task.RevokeMemberArgs{PeerID: peerID, Dangerous: true}, stdout, stderr)
}

func runTaskKind(opt globalOptions, kind string, args any, stdout, stderr io.Writer) int {
	ctx, cancel := taskContextForKind(kind, args)
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
	if opt.Redact {
		env.Facts = redactFacts(env.Facts)
		env.Suggestions = redactSuggestions(env.Suggestions)
	}

	if strings.TrimSpace(opt.ReportPath) != "" {
		reportCtx, cancelReport := context.WithTimeout(context.Background(), 5*time.Second)
		err := exportTaskReport(reportCtx, c, finalTask.ID, opt.ReportPath, opt.Redact)
		cancelReport()
		if err != nil {
			return exitWithFailure(opt, stdout, stderr, kind, created.ID, failureOutput{
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
		}
	}
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

func taskContextForKind(kind string, args any) (context.Context, context.CancelFunc) {
	defaultTimeout := 2 * time.Minute
	codeValue := ""
	switch strings.TrimSpace(kind) {
	case "join":
		if v, ok := args.(task.JoinArgs); ok {
			codeValue = strings.TrimSpace(v.Code)
		}
	case "approve":
		if v, ok := args.(task.ApproveArgs); ok {
			codeValue = strings.TrimSpace(v.Code)
		}
	}

	if codeValue != "" {
		if decoded, err := controlplane.DecodeInviteCodeV0(codeValue); err == nil {
			expiresAt := time.UnixMilli(decoded.ExpiresAtUnixMs).UTC().Add(30 * time.Second)
			if time.Until(expiresAt) > 0 {
				return context.WithDeadline(context.Background(), expiresAt)
			}
		}
	}
	return context.WithTimeout(context.Background(), defaultTimeout)
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
	if opt.Redact {
		f.Facts = redactFacts(f.Facts)
		f.Suggestions = redactSuggestions(f.Suggestions)
	}
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
