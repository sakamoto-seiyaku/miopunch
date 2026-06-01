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

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
	"github.com/miopunch/miopunch/internal/shellproto"
)

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
	case "status":
		return statusCmd(ctx, args[1:], stdout, stderr)
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
  miopunch-poc-e2e status [flags]

Commands:
  sh-attach:
    --localapi unix:/run/miopunch/localapi.sock
    --peer-id <peer-id>
    --target <target>       (default: local)
    --session <name>        (default: main)
    --send <bytes>
    --expect <substring>
    --timeout <duration>    (default: 10s)
    --hold <duration>       keep the shell channel open after observing --expect
  status:
    --localapi unix:/run/miopunch/localapi.sock

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
	OK             bool           `json:"ok"`
	ShellSessionID string         `json:"shell_session_id,omitempty"`
	PeerID         string         `json:"peer_id,omitempty"`
	Target         string         `json:"target,omitempty"`
	Session        string         `json:"session,omitempty"`
	SentBytes      int            `json:"sent_bytes,omitempty"`
	ObservedBytes  int            `json:"observed_bytes,omitempty"`
	Expect         string         `json:"expect,omitempty"`
	Stage          string         `json:"stage,omitempty"`
	ReasonCode     poc.ReasonCode `json:"reason_code,omitempty"`
	ExitCode       poc.ExitCode   `json:"exit_code,omitempty"`
	Error          string         `json:"error,omitempty"`
}

type statusConfig struct {
	LocalAPI string
	Timeout  time.Duration
}

type statusResult struct {
	OK         bool           `json:"ok"`
	Mode       string         `json:"mode,omitempty"`
	Version    string         `json:"version,omitempty"`
	Stage      string         `json:"stage,omitempty"`
	ReasonCode poc.ReasonCode `json:"reason_code,omitempty"`
	ExitCode   poc.ExitCode   `json:"exit_code,omitempty"`
	Error      string         `json:"error,omitempty"`
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

	client, err := localapi.NewClient(addr)
	if err != nil {
		return writeShAttachFailure(stdout, stderr, cfg, "", "setup", poc.ReasonCodeInternal, poc.ExitCodeInternal, err)
	}
	if err := client.ProbeStatus(runCtx); err != nil {
		return writeShAttachFailure(stdout, stderr, cfg, "", "setup", poc.ReasonCodeDaemonNotRunning, poc.ExitCodeUnavailable, err)
	}

	result, err := client.Action(runCtx, "sh", pocruntime.ShellArgs{
		PeerID:  cfg.PeerID,
		Target:  cfg.Target,
		Session: cfg.Session,
	})
	if err != nil {
		reason, exitCode, message := localAPIFailure(err)
		return writeShAttachFailure(stdout, stderr, cfg, "", "create_shell", reason, exitCode, message)
	}
	if strings.TrimSpace(result.ShellSessionID) == "" {
		return writeShAttachFailure(stdout, stderr, cfg, "", "create_shell", poc.ReasonCodeInternal, poc.ExitCodeInternal, errors.New("missing shell_session_id"))
	}

	stream, err := client.DialShell(runCtx, result.ShellSessionID)
	if err != nil {
		return writeShAttachFailure(stdout, stderr, cfg, result.ShellSessionID, "attach_shell", poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, err)
	}
	defer func() { _ = stream.Close() }()

	if err := shellproto.WriteFrame(stream, shellproto.KindData, []byte(cfg.Send)); err != nil {
		return writeShAttachFailure(stdout, stderr, cfg, result.ShellSessionID, "send", poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, err)
	}

	observed, err := waitForOutput(runCtx, stream, []byte(cfg.Expect))
	if err != nil {
		if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
			return writeShAttachFailure(stdout, stderr, cfg, result.ShellSessionID, "read", poc.ReasonCodeTimeout, poc.ExitCodeTimeout, err)
		}
		return writeShAttachFailure(stdout, stderr, cfg, result.ShellSessionID, "read", poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, err)
	}

	if cfg.Hold > 0 {
		if err := hold(runCtx, cfg.Hold); err != nil {
			return writeShAttachFailure(stdout, stderr, cfg, result.ShellSessionID, "hold", poc.ReasonCodeTimeout, poc.ExitCodeTimeout, err)
		}
	}

	_ = stream.Close()
	writeShAttachJSON(stdout, shAttachResult{
		OK:             true,
		ShellSessionID: result.ShellSessionID,
		PeerID:         cfg.PeerID,
		Target:         cfg.Target,
		Session:        cfg.Session,
		SentBytes:      len([]byte(cfg.Send)),
		ObservedBytes:  len(observed),
		Expect:         cfg.Expect,
		Stage:          string(result.Stage),
		ReasonCode:     result.ReasonCode,
		ExitCode:       result.ExitCode,
	})
	return 0
}

func statusCmd(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	cfg, err := parseStatusFlags(args, stderr)
	if err != nil {
		if errors.Is(err, flag.ErrHelp) {
			return 0
		}
		return writeStatusFailure(stdout, stderr, "args", poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, err)
	}

	runCtx, cancel := context.WithTimeout(ctx, cfg.Timeout)
	defer cancel()

	addr, err := parseLocalAPIAddr(cfg.LocalAPI)
	if err != nil {
		return writeStatusFailure(stdout, stderr, "setup", poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest, err)
	}
	client, err := localapi.NewClient(addr)
	if err != nil {
		return writeStatusFailure(stdout, stderr, "setup", poc.ReasonCodeInternal, poc.ExitCodeInternal, err)
	}

	status, err := client.GetStatus(runCtx)
	if err != nil {
		reason, exitCode, message := localAPIFailure(err)
		return writeStatusFailure(stdout, stderr, "status", reason, exitCode, message)
	}

	writeStatusJSON(stdout, statusResult{
		OK:         true,
		Mode:       string(status.Mode),
		Version:    status.Version,
		ReasonCode: poc.ReasonCodeOK,
		ExitCode:   poc.ExitCodeOK,
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
	fs.StringVar(&cfg.Send, "send", "", "bytes to send as shell input")
	fs.StringVar(&cfg.Expect, "expect", "", "substring expected in shell output")
	fs.DurationVar(&cfg.Timeout, "timeout", cfg.Timeout, "overall timeout")
	fs.DurationVar(&cfg.Hold, "hold", 0, "keep shell open after observing --expect")
	return cfg, fs.Parse(args)
}

func parseStatusFlags(args []string, stderr io.Writer) (statusConfig, error) {
	cfg := statusConfig{
		LocalAPI: "unix:/run/miopunch/localapi.sock",
		Timeout:  5 * time.Second,
	}
	fs := flag.NewFlagSet("status", flag.ContinueOnError)
	fs.SetOutput(stderr)
	fs.StringVar(&cfg.LocalAPI, "localapi", cfg.LocalAPI, "LocalAPI address")
	fs.DurationVar(&cfg.Timeout, "timeout", cfg.Timeout, "request timeout")
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
	return localapi.ParseAddr(value)
}

func localAPIFailure(err error) (poc.ReasonCode, poc.ExitCode, error) {
	var apiErr *localapi.APIError
	if errors.As(err, &apiErr) {
		response := apiErr.Response
		message := strings.TrimSpace(response.Message)
		if message == "" {
			message = apiErr.Error()
		}
		return response.ReasonCode, response.ExitCode, errors.New(message)
	}
	return poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable, err
}

func waitForOutput(ctx context.Context, stream io.ReadWriteCloser, expect []byte) ([]byte, error) {
	var observed []byte
	if stream == nil {
		return nil, errors.New("nil shell stream")
	}

	doneCh := make(chan struct{})
	defer close(doneCh)
	go func() {
		select {
		case <-doneCh:
		case <-ctx.Done():
			_ = stream.Close()
		}
	}()

	for {
		kind, payload, err := shellproto.ReadFrame(stream)
		if err != nil {
			if ctx.Err() != nil {
				return observed, ctx.Err()
			}
			return observed, err
		}
		if kind != shellproto.KindData {
			continue
		}
		observed = append(observed, payload...)
		if bytes.Contains(observed, expect) {
			return observed, nil
		}
	}
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

func writeShAttachFailure(stdout, stderr io.Writer, cfg shAttachConfig, shellSessionID string, stage string, reason poc.ReasonCode, exit poc.ExitCode, err error) int {
	fmt.Fprintf(stderr, "sh-attach %s: %v\n", stage, err)
	writeShAttachJSON(stdout, shAttachResult{
		OK:             false,
		ShellSessionID: shellSessionID,
		PeerID:         cfg.PeerID,
		Target:         cfg.Target,
		Session:        cfg.Session,
		Expect:         cfg.Expect,
		Stage:          stage,
		ReasonCode:     reason,
		ExitCode:       exit,
		Error:          err.Error(),
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

func writeStatusFailure(stdout, stderr io.Writer, stage string, reason poc.ReasonCode, exit poc.ExitCode, err error) int {
	fmt.Fprintf(stderr, "status %s: %v\n", stage, err)
	writeStatusJSON(stdout, statusResult{
		OK:         false,
		Stage:      stage,
		ReasonCode: reason,
		ExitCode:   exit,
		Error:      err.Error(),
	})
	return int(exit)
}

func writeStatusJSON(w io.Writer, result statusResult) {
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(result); err != nil {
		fmt.Fprintf(w, `{"ok":false,"error":%q}`+"\n", err.Error())
	}
}
