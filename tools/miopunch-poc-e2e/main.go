package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/gorilla/websocket"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/task"
)

const shSubprotocolV0 = "miopunch.sh.v0"

func main() {
	os.Exit(run(os.Args[1:], os.Stdout, os.Stderr))
}

func run(args []string, stdout, stderr io.Writer) int {
	if len(args) < 1 {
		usage(stderr)
		return 2
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	switch args[0] {
	case "sh-attach":
		return shAttachCmd(ctx, args[1:], stdout, stderr)
	case "-h", "--help", "help":
		usage(stdout)
		return 0
	default:
		fmt.Fprintf(stderr, "unknown command: %s\n\n", args[0])
		usage(stderr)
		return 2
	}
}

func usage(w io.Writer) {
	fmt.Fprint(w, `miopunch-poc-e2e (repo-local POC e2e helper)

Usage:
  miopunch-poc-e2e sh-attach [flags]

Commands:
  sh-attach:
    --localapi unix:/run/miopunch/localapi.sock
    --peer-id <peer-id>
    --target <target>       (default: local)
    --session <name>        (default: main)
    --send <bytes>
    --expect <substring>
    --timeout <duration>    (default: 10s)
    --hold <duration>       keep the websocket open after observing --expect

Output:
  Always emits a single-line JSON result to stdout.
`)
}

type shAttachConfig struct {
	LocalAPI string
	PeerID   string
	Target   string
	Session  string
	Send     string
	Expect   string
	Timeout  time.Duration
	Hold     time.Duration
}

type shAttachResult struct {
	OK            bool           `json:"ok"`
	TaskID        string         `json:"task_id,omitempty"`
	PeerID        string         `json:"peer_id,omitempty"`
	Target        string         `json:"target,omitempty"`
	Session       string         `json:"session,omitempty"`
	SentBytes     int            `json:"sent_bytes,omitempty"`
	ObservedBytes int            `json:"observed_bytes,omitempty"`
	Expect        string         `json:"expect,omitempty"`
	Stage         string         `json:"stage,omitempty"`
	ReasonCode    poc.ReasonCode `json:"reason_code,omitempty"`
	ExitCode      poc.ExitCode   `json:"exit_code,omitempty"`
	Error         string         `json:"error,omitempty"`
}

func shAttachCmd(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	cfg, err := parseShAttachFlags(args, stderr)
	if err != nil {
		if errors.Is(err, flag.ErrHelp) {
			return 0
		}
		return writeShAttachFailure(stdout, stderr, cfg, "", "args", poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, err)
	}
	if err := validateShAttachConfig(cfg); err != nil {
		return writeShAttachFailure(stdout, stderr, cfg, "", "args", poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, err)
	}

	decodedSend, err := decodeEscapes(cfg.Send)
	if err != nil {
		return writeShAttachFailure(stdout, stderr, cfg, "", "args", poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, err)
	}
	cfg.Send = decodedSend

	runCtx, cancel := context.WithTimeout(ctx, cfg.Timeout)
	defer cancel()

	addr, err := parseLocalAPIAddr(cfg.LocalAPI)
	if err != nil {
		return writeShAttachFailure(stdout, stderr, cfg, "", "setup", poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, err)
	}

	c, err := localapi.NewClient(addr)
	if err != nil {
		return writeShAttachFailure(stdout, stderr, cfg, "", "setup", poc.ReasonCodeInternal, poc.ExitCodeInternal, err)
	}
	if err := c.ProbeStatus(runCtx); err != nil {
		return writeShAttachFailure(stdout, stderr, cfg, "", "setup", poc.ReasonCodeDaemonNotRunning, poc.ExitCodeUnavailable, err)
	}

	created, err := c.CreateTask(runCtx, "sh_attach", task.ShAttachArgs{
		PeerID:  cfg.PeerID,
		Target:  cfg.Target,
		Session: cfg.Session,
	})
	if err != nil {
		return writeShAttachFailure(stdout, stderr, cfg, "", "create_task", poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, err)
	}

	conn, resp, err := c.DialTaskWS(runCtx, created.ID)
	if resp != nil && resp.Body != nil {
		_ = resp.Body.Close()
	}
	if err != nil {
		if t, ok := getDoneTask(runCtx, c, created.ID); ok && t.ExitCode != poc.ExitCodeOK {
			return writeShAttachFailure(stdout, stderr, cfg, created.ID, string(t.Stage), t.ReasonCode, t.ExitCode, fmt.Errorf("websocket dial: %w", err))
		}
		return writeShAttachFailure(stdout, stderr, cfg, created.ID, "websocket", poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, err)
	}
	defer func() { _ = conn.Close() }()

	if conn.Subprotocol() != shSubprotocolV0 {
		err := fmt.Errorf("websocket subprotocol = %q, want %q", conn.Subprotocol(), shSubprotocolV0)
		return writeShAttachFailure(stdout, stderr, cfg, created.ID, "websocket", poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, err)
	}

	if err := conn.WriteMessage(websocket.BinaryMessage, []byte(cfg.Send)); err != nil {
		if t, ok := getDoneTask(runCtx, c, created.ID); ok && t.ExitCode != poc.ExitCodeOK {
			return writeShAttachFailure(stdout, stderr, cfg, created.ID, string(t.Stage), t.ReasonCode, t.ExitCode, fmt.Errorf("websocket send: %w", err))
		}
		return writeShAttachFailure(stdout, stderr, cfg, created.ID, "send", poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, err)
	}

	observed, err := waitForOutputWithTask(runCtx, c, created.ID, conn, []byte(cfg.Expect))
	if err != nil {
		var taskErr *taskFailureError
		if errors.As(err, &taskErr) {
			t := taskErr.Task
			return writeShAttachFailure(stdout, stderr, cfg, created.ID, string(t.Stage), t.ReasonCode, t.ExitCode, err)
		}
		if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
			return writeShAttachFailure(stdout, stderr, cfg, created.ID, "read", poc.ReasonCodeTimeout, poc.ExitCodeTimeout, err)
		}
		if t, ok := getDoneTask(runCtx, c, created.ID); ok && t.ExitCode != poc.ExitCodeOK {
			return writeShAttachFailure(stdout, stderr, cfg, created.ID, string(t.Stage), t.ReasonCode, t.ExitCode, fmt.Errorf("websocket read: %w", err))
		}
		return writeShAttachFailure(stdout, stderr, cfg, created.ID, "read", poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, err)
	}

	if cfg.Hold > 0 {
		if err := hold(runCtx, cfg.Hold); err != nil {
			return writeShAttachFailure(stdout, stderr, cfg, created.ID, "hold", poc.ReasonCodeTimeout, poc.ExitCodeTimeout, err)
		}
	}

	_ = conn.WriteControl(
		websocket.CloseMessage,
		websocket.FormatCloseMessage(websocket.CloseNormalClosure, "miopunch-poc-e2e done"),
		time.Now().Add(2*time.Second),
	)
	_ = conn.Close()

	finalTask, err := waitTaskDone(runCtx, c, created.ID)
	if err != nil {
		return writeShAttachFailure(stdout, stderr, cfg, created.ID, "task_done", poc.ReasonCodeTimeout, poc.ExitCodeTimeout, err)
	}
	if finalTask.ExitCode != poc.ExitCodeOK {
		err := fmt.Errorf("task failed: reason_code=%s exit_code=%d", finalTask.ReasonCode, finalTask.ExitCode)
		return writeShAttachFailure(stdout, stderr, cfg, created.ID, string(finalTask.Stage), finalTask.ReasonCode, finalTask.ExitCode, err)
	}

	writeShAttachJSON(stdout, shAttachResult{
		OK:            true,
		TaskID:        created.ID,
		PeerID:        cfg.PeerID,
		Target:        cfg.Target,
		Session:       cfg.Session,
		SentBytes:     len([]byte(cfg.Send)),
		ObservedBytes: len(observed),
		Expect:        cfg.Expect,
		Stage:         string(finalTask.Stage),
		ReasonCode:    finalTask.ReasonCode,
		ExitCode:      finalTask.ExitCode,
	})
	return 0
}

func parseShAttachFlags(args []string, stderr io.Writer) (shAttachConfig, error) {
	cfg := shAttachConfig{
		LocalAPI: "unix:/run/miopunch/localapi.sock",
		Target:   "local",
		Session:  "main",
		Timeout:  10 * time.Second,
	}

	fs := flag.NewFlagSet("sh-attach", flag.ContinueOnError)
	fs.SetOutput(stderr)
	fs.StringVar(&cfg.LocalAPI, "localapi", cfg.LocalAPI, "LocalAPI address")
	fs.StringVar(&cfg.PeerID, "peer-id", "", "peer id to attach")
	fs.StringVar(&cfg.Target, "target", cfg.Target, "shell target")
	fs.StringVar(&cfg.Session, "session", cfg.Session, "tmux session")
	fs.StringVar(&cfg.Send, "send", "", "bytes to send as a websocket binary message")
	fs.StringVar(&cfg.Expect, "expect", "", "substring expected in websocket binary output")
	fs.DurationVar(&cfg.Timeout, "timeout", cfg.Timeout, "overall timeout")
	fs.DurationVar(&cfg.Hold, "hold", 0, "keep websocket open after observing --expect")
	return cfg, fs.Parse(args)
}

func validateShAttachConfig(cfg shAttachConfig) error {
	if strings.TrimSpace(cfg.LocalAPI) == "" {
		return errors.New("missing --localapi")
	}
	if strings.TrimSpace(cfg.PeerID) == "" {
		return errors.New("missing --peer-id")
	}
	if cfg.Send == "" {
		return errors.New("missing --send")
	}
	if cfg.Expect == "" {
		return errors.New("missing --expect")
	}
	if cfg.Timeout <= 0 {
		return errors.New("--timeout must be positive")
	}
	if cfg.Hold < 0 {
		return errors.New("--hold must be non-negative")
	}
	return nil
}

func decodeEscapes(value string) (string, error) {
	if value == "" {
		return "", nil
	}

	quoted := `"` + strings.ReplaceAll(value, `"`, `\"`) + `"`
	decoded, err := strconv.Unquote(quoted)
	if err != nil {
		return "", fmt.Errorf("invalid escapes: %w", err)
	}
	return decoded, nil
}

func parseLocalAPIAddr(value string) (localapi.Addr, error) {
	v := strings.TrimSpace(value)
	switch {
	case strings.HasPrefix(v, "unix:"):
		path := strings.TrimSpace(strings.TrimPrefix(v, "unix:"))
		if path == "" {
			return localapi.Addr{}, errors.New("empty unix socket path")
		}
		return localapi.Addr{Transport: localapi.TransportUnix, Path: path}, nil
	case strings.HasPrefix(v, "npipe:"):
		path := strings.TrimSpace(strings.TrimPrefix(v, "npipe:"))
		if path == "" {
			return localapi.Addr{}, errors.New("empty npipe path")
		}
		return localapi.Addr{Transport: localapi.TransportNpipe, Path: path}, nil
	default:
		return localapi.Addr{}, fmt.Errorf("unsupported localapi address: %q", value)
	}
}

func waitForOutput(ctx context.Context, conn *websocket.Conn, expect []byte) ([]byte, error) {
	var observed []byte

	if conn == nil {
		return nil, errors.New("nil websocket conn")
	}

	if deadline, ok := ctx.Deadline(); ok {
		_ = conn.SetReadDeadline(deadline)
	}

	stopCh := make(chan struct{})
	defer close(stopCh)
	go func() {
		select {
		case <-stopCh:
		case <-ctx.Done():
			_ = conn.Close()
		}
	}()

	for {
		msgType, payload, err := conn.ReadMessage()
		if err != nil {
			if ctx.Err() != nil {
				return observed, ctx.Err()
			}
			return observed, err
		}
		if msgType != websocket.BinaryMessage {
			continue
		}

		observed = append(observed, payload...)
		if bytes.Contains(observed, expect) {
			return observed, nil
		}
	}
}

type taskFailureError struct {
	Task task.Task
}

func (e *taskFailureError) Error() string {
	return fmt.Sprintf("task failed: stage=%s reason_code=%s exit_code=%d", e.Task.Stage, e.Task.ReasonCode, e.Task.ExitCode)
}

func waitForOutputWithTask(ctx context.Context, c *localapi.Client, taskID string, conn *websocket.Conn, expect []byte) ([]byte, error) {
	if c == nil {
		return waitForOutput(ctx, conn, expect)
	}

	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	type outputResult struct {
		observed []byte
		err      error
	}

	outputCh := make(chan outputResult, 1)
	taskCh := make(chan task.Task, 1)

	go func() {
		observed, err := waitForOutput(ctx, conn, expect)
		outputCh <- outputResult{observed: observed, err: err}
	}()

	go func() {
		ticker := time.NewTicker(100 * time.Millisecond)
		defer ticker.Stop()

		for {
			t, ok := getDoneTask(ctx, c, taskID)
			if ok && t.ExitCode != poc.ExitCodeOK {
				select {
				case taskCh <- t:
				default:
				}
				return
			}

			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
			}
		}
	}()

	select {
	case t := <-taskCh:
		cancel()
		_ = conn.Close()
		return nil, &taskFailureError{Task: t}
	case res := <-outputCh:
		if res.err != nil {
			if t, ok := getDoneTask(ctx, c, taskID); ok && t.ExitCode != poc.ExitCodeOK {
				return res.observed, &taskFailureError{Task: t}
			}
		}
		return res.observed, res.err
	case <-ctx.Done():
		_ = conn.Close()
		return nil, ctx.Err()
	}
}

func getDoneTask(ctx context.Context, c *localapi.Client, taskID string) (task.Task, bool) {
	if c == nil || strings.TrimSpace(taskID) == "" {
		return task.Task{}, false
	}

	t, err := c.GetTask(ctx, taskID)
	if err != nil || t.Status != task.StatusDone {
		return task.Task{}, false
	}
	return t, true
}

func hold(ctx context.Context, d time.Duration) error {
	timer := time.NewTimer(d)
	defer timer.Stop()

	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-timer.C:
		return nil
	}
}

func waitTaskDone(ctx context.Context, c *localapi.Client, taskID string) (task.Task, error) {
	for {
		t, err := c.GetTask(ctx, taskID)
		if err == nil && t.Status == task.StatusDone {
			return t, nil
		}
		if err != nil && ctx.Err() != nil {
			return task.Task{}, ctx.Err()
		}

		select {
		case <-ctx.Done():
			return task.Task{}, ctx.Err()
		case <-time.After(50 * time.Millisecond):
		}
	}
}

func writeShAttachFailure(stdout, stderr io.Writer, cfg shAttachConfig, taskID string, stage string, reason poc.ReasonCode, exit poc.ExitCode, err error) int {
	fmt.Fprintf(stderr, "sh-attach %s: %v\n", stage, err)
	writeShAttachJSON(stdout, shAttachResult{
		OK:         false,
		TaskID:     taskID,
		PeerID:     cfg.PeerID,
		Target:     cfg.Target,
		Session:    cfg.Session,
		Expect:     cfg.Expect,
		Stage:      stage,
		ReasonCode: reason,
		ExitCode:   exit,
		Error:      err.Error(),
	})
	return int(exit)
}

func writeShAttachJSON(w io.Writer, result shAttachResult) {
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(result); err != nil {
		fmt.Fprintf(w, `{"ok":false,"error":%q}`+"\n", err.Error())
	}
}
