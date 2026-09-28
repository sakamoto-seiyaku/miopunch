package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"strconv"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/shelltarget"
)

func runDebugConPTYSmoke(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	fs := flag.NewFlagSet("debug-conpty-smoke", flag.ContinueOnError)
	fs.SetOutput(stderr)
	timeout := fs.Duration("timeout", 6*time.Second, "overall smoke timeout")
	writeDelay := fs.Duration("write_delay", 2*time.Second, "delay before optional write")
	writeInput := fs.String("write", "", "optional input to write after --write_delay; supports \\r, \\n, \\t, \\x1b")
	cols := fs.Int("cols", 80, "initial ConPTY columns")
	rows := fs.Int("rows", 24, "initial ConPTY rows")
	if err := fs.Parse(args); err != nil {
		return debugConPTYSmokeUsage(opt, stdout, stderr, err.Error())
	}

	req, label, err := debugConPTYSmokeRequest(fs.Args(), *timeout, *writeDelay, *writeInput, *cols, *rows)
	if err != nil {
		return debugConPTYSmokeUsage(opt, stdout, stderr, err.Error())
	}

	ctx, cancel := context.WithTimeout(context.Background(), *timeout+3*time.Second)
	defer cancel()

	result := shelltarget.RunConPTYSmoke(ctx, req)
	if opt.Format == outputFormatJSON {
		enc := json.NewEncoder(stdout)
		enc.SetIndent("", "  ")
		_ = enc.Encode(result)
	} else {
		writeDebugConPTYSmokeResult(stdout, label, result)
	}

	if result.StartErr != "" || result.ReadN == 0 {
		return int(poc.ExitCodeUnavailable)
	}
	return int(poc.ExitCodeOK)
}

func debugConPTYSmokeRequest(args []string, timeout, writeDelay time.Duration, writeInput string, cols, rows int) (shelltarget.ConPTYSmokeRequest, string, error) {
	if len(args) == 0 {
		return shelltarget.ConPTYSmokeRequest{}, "", fmt.Errorf("missing smoke case")
	}
	label := strings.TrimSpace(args[0])
	req := shelltarget.ConPTYSmokeRequest{
		Timeout:    timeout,
		WriteDelay: writeDelay,
		Cols:       cols,
		Rows:       rows,
	}

	switch label {
	case "cmd":
		req.Application = "cmd.exe"
		req.Args = []string{"/d", "/c", "echo __MIO_CONPTY_CMD__"}
	case "ssh-printf":
		if len(args) < 2 {
			return shelltarget.ConPTYSmokeRequest{}, "", fmt.Errorf("ssh-printf requires ssh target")
		}
		req.Application = "ssh"
		req.Args = []string{"-tt", args[1], "printf '__MIO_CONPTY_SSH__\\r\\n'; sleep 1"}
	case "ssh-tty":
		if len(args) < 2 {
			return shelltarget.ConPTYSmokeRequest{}, "", fmt.Errorf("ssh-tty requires ssh target")
		}
		req.Application = "ssh"
		req.Args = []string{"-tt", args[1], "printf '__MIO_CONPTY_TTY__\\r\\n'; stty size; tty; sleep 1"}
	case "ssh-tmux":
		if len(args) < 2 {
			return shelltarget.ConPTYSmokeRequest{}, "", fmt.Errorf("ssh-tmux requires ssh target")
		}
		session := "main"
		if len(args) >= 3 {
			session = strings.TrimSpace(args[2])
		}
		req.Application = "ssh"
		req.Args = []string{"-tt", args[1], "tmux", "new", "-A", "-s", session}
		if writeInput == "" {
			writeInput = `\r`
		}
	case "raw":
		if len(args) < 2 {
			return shelltarget.ConPTYSmokeRequest{}, "", fmt.Errorf("raw requires application")
		}
		req.Application = args[1]
		req.Args = append([]string(nil), args[2:]...)
	default:
		return shelltarget.ConPTYSmokeRequest{}, "", fmt.Errorf("unknown smoke case: %s", label)
	}

	input, err := decodeConPTYSmokeInput(writeInput)
	if err != nil {
		return shelltarget.ConPTYSmokeRequest{}, "", err
	}
	req.Input = input
	return req, label, nil
}

func decodeConPTYSmokeInput(value string) ([]byte, error) {
	if value == "" {
		return nil, nil
	}
	quoted := `"` + strings.ReplaceAll(value, `"`, `\"`) + `"`
	decoded, err := strconv.Unquote(quoted)
	if err != nil {
		return nil, fmt.Errorf("invalid --write escape sequence: %w", err)
	}
	return []byte(decoded), nil
}

func writeDebugConPTYSmokeResult(w io.Writer, label string, r shelltarget.ConPTYSmokeResult) {
	fmt.Fprintf(w, "case=%s\n", label)
	fmt.Fprintf(w, "application=%s\n", r.Application)
	fmt.Fprintf(w, "args=%q\n", r.Args)
	fmt.Fprintf(w, "command_line=%s\n", r.CommandLine)
	fmt.Fprintf(w, "started=%t\n", r.Started)
	if r.PID != 0 {
		fmt.Fprintf(w, "pid=%d\n", r.PID)
	}
	if r.StartErr != "" {
		fmt.Fprintf(w, "start_err=%s\n", r.StartErr)
	}
	fmt.Fprintf(w, "timeout_ms=%d\n", r.TimeoutMS)
	fmt.Fprintf(w, "duration_ms=%d\n", r.DurationMS)
	fmt.Fprintf(w, "read_returned=%t\n", r.ReadReturned)
	fmt.Fprintf(w, "read_timed_out=%t\n", r.ReadTimedOut)
	fmt.Fprintf(w, "read_after_close=%t\n", r.ReadAfterClose)
	fmt.Fprintf(w, "read_chunks=%d\n", r.ReadChunks)
	fmt.Fprintf(w, "read_n=%d\n", r.ReadN)
	if r.ReadErr != "" {
		fmt.Fprintf(w, "read_err=%s\n", r.ReadErr)
	}
	if r.ReadAfterMS > 0 {
		fmt.Fprintf(w, "read_after_ms=%d\n", r.ReadAfterMS)
	}
	if r.ReadLastAfterMS > 0 {
		fmt.Fprintf(w, "read_last_after_ms=%d\n", r.ReadLastAfterMS)
	}
	if r.PreviewText != "" {
		fmt.Fprintf(w, "preview_text=%s\n", r.PreviewText)
	}
	if r.PreviewHex != "" {
		fmt.Fprintf(w, "preview_hex=%s\n", r.PreviewHex)
	}
	fmt.Fprintf(w, "write_attempted=%t\n", r.WriteAttempted)
	if r.WriteAttempted {
		fmt.Fprintf(w, "write_requested_bytes=%d\n", r.WriteRequestedBytes)
		fmt.Fprintf(w, "write_returned=%t\n", r.WriteReturned)
		fmt.Fprintf(w, "write_n=%d\n", r.WriteN)
		if r.WriteErr != "" {
			fmt.Fprintf(w, "write_err=%s\n", r.WriteErr)
		}
		if r.WriteAfterMS > 0 {
			fmt.Fprintf(w, "write_after_ms=%d\n", r.WriteAfterMS)
		}
	}
	fmt.Fprintf(w, "wait_returned=%t\n", r.WaitReturned)
	if r.WaitErr != "" {
		fmt.Fprintf(w, "wait_err=%s\n", r.WaitErr)
	}
	if r.WaitAfterMS > 0 {
		fmt.Fprintf(w, "wait_after_ms=%d\n", r.WaitAfterMS)
	}
}

func debugConPTYSmokeUsage(opt globalOptions, stdout, stderr io.Writer, msg string) int {
	return exitWithFailure(opt, stdout, stderr, "debug-conpty-smoke", "", failureOutput{
		Stage:      "cli",
		ReasonCode: poc.ReasonCodeBadRequest,
		ExitCode:   poc.ExitCodeBadRequest,
		Facts: []poc.Fact{
			{Message: msg},
			{Message: "usage: miopunch debug-conpty-smoke [--timeout 6s] <cmd|ssh-printf|ssh-tty|ssh-tmux|raw> [args...]"},
		},
		Suggestions: []poc.Suggestion{
			{Message: `try: miopunch debug-conpty-smoke cmd`},
			{Message: `try: miopunch debug-conpty-smoke ssh-printf ale`},
			{Message: `try: miopunch debug-conpty-smoke ssh-tmux ale main`},
		},
	})
}
