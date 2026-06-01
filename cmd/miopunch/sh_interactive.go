package main

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	"strings"
	"sync"
	"time"

	"golang.org/x/term"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func runSh(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	if opt.Format != outputFormatHuman {
		return exitWithFailure(opt, stdout, stderr, "sh", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts:      []poc.Fact{{Message: "--format json is not supported for interactive sh"}},
			Suggestions: []poc.Suggestion{
				{Message: "retry without --format json"},
			},
		})
	}

	peerID := ""
	target := ""
	sessionName := "main"
	p2pNetwork := "auto"

	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch {
		case arg == "-s" || arg == "-session" || arg == "--session":
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
			i++
			sessionName = strings.TrimSpace(args[i])
		case strings.HasPrefix(arg, "-s="):
			sessionName = strings.TrimSpace(strings.TrimPrefix(arg, "-s="))
		case strings.HasPrefix(arg, "--session="):
			sessionName = strings.TrimSpace(strings.TrimPrefix(arg, "--session="))
		case strings.HasPrefix(arg, "-session="):
			sessionName = strings.TrimSpace(strings.TrimPrefix(arg, "-session="))
		case arg == "-u":
			p2pNetwork = "udp_only"
		case arg == "-t":
			p2pNetwork = "tcp_only"
		case arg == "--p2p-network":
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
			i++
			p2pNetwork = strings.TrimSpace(args[i])
		case strings.HasPrefix(arg, "--p2p-network="):
			p2pNetwork = strings.TrimSpace(strings.TrimPrefix(arg, "--p2p-network="))
		case strings.HasPrefix(arg, "-"):
			return exitWithFailure(opt, stdout, stderr, "sh", "", failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Facts:      []poc.Fact{{Message: "unknown arg: " + arg}},
				Suggestions: []poc.Suggestion{
					{Message: "use: miopunch sh <peer_id> [target] [-s session] [-u|-t|--p2p-network ...]"},
				},
			})
		default:
			if peerID == "" {
				peerID = arg
			} else if target == "" {
				target = arg
			} else {
				return exitWithFailure(opt, stdout, stderr, "sh", "", failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "unexpected extra arg: " + arg}},
					Suggestions: []poc.Suggestion{
						{Message: "use: miopunch sh <peer_id> [target] [-s session]"},
					},
				})
			}
		}
	}

	if strings.TrimSpace(peerID) == "" {
		return exitWithFailure(opt, stdout, stderr, "sh", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts:      []poc.Fact{{Message: "missing peer_id"}},
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

	return runShellInteractive(opt, pocruntime.ShellArgs{
		PeerID:     peerID,
		Target:     target,
		Session:    sessionName,
		P2PNetwork: string(network),
	}, stdout, stderr)
}

type shellFrame struct {
	kind    string
	payload []byte
}

type shellClient interface {
	Action(context.Context, string, any) (pocruntime.ActionResult, error)
	DialShell(context.Context, string) (io.ReadWriteCloser, error)
}

type localAPIShellClient struct {
	client *localapi.Client
}

func (c localAPIShellClient) Action(ctx context.Context, action string, args any) (pocruntime.ActionResult, error) {
	return c.client.Action(ctx, action, args)
}

func (c localAPIShellClient) DialShell(ctx context.Context, shellSessionID string) (io.ReadWriteCloser, error) {
	return c.client.DialShell(ctx, shellSessionID)
}

type shellInteractiveDeps struct {
	connect     func(context.Context, string) (shellClient, error)
	stdin       io.Reader
	stdinFD     func() int
	isTerminal  func(int) bool
	makeRaw     func(int) (*term.State, error)
	restoreTerm func(int, *term.State) error
	getSize     func(int) (int, int, error)
	watchResize func(context.Context, int, func(int, int))
}

func defaultShellInteractiveDeps() shellInteractiveDeps {
	return shellInteractiveDeps{
		connect: func(ctx context.Context, override string) (shellClient, error) {
			client, _, err := connectLocalAPI(ctx, override)
			if err != nil {
				return nil, err
			}
			return localAPIShellClient{client: client}, nil
		},
		stdin:       os.Stdin,
		stdinFD:     func() int { return int(os.Stdin.Fd()) },
		isTerminal:  term.IsTerminal,
		makeRaw:     term.MakeRaw,
		restoreTerm: term.Restore,
		getSize:     term.GetSize,
		watchResize: watchResize,
	}
}

func runShellInteractive(opt globalOptions, args pocruntime.ShellArgs, stdout, stderr io.Writer) int {
	return runShellInteractiveWithDeps(opt, args, stdout, stderr, defaultShellInteractiveDeps())
}

func runShellInteractiveWithDeps(
	opt globalOptions,
	args pocruntime.ShellArgs,
	stdout io.Writer,
	stderr io.Writer,
	deps shellInteractiveDeps,
) int {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	apiCtx, cancelAPI := context.WithTimeout(ctx, 5*time.Second)
	defer cancelAPI()

	client, err := deps.connect(apiCtx, opt.LocalAPIOverride)
	if err != nil {
		return exitWithError(opt, stdout, stderr, "sh", "", err)
	}

	actionCtx, cancelAction := context.WithTimeout(ctx, actionTimeout("sh"))
	defer cancelAction()

	result, err := client.Action(actionCtx, "sh", args)
	if err != nil {
		return exitWithError(opt, stdout, stderr, "sh", "", err)
	}
	if err := exportReportMarkdown(opt.ReportPath, result.ReportMarkdown, opt.Redact); err != nil {
		return exitWithFailure(opt, stdout, stderr, "sh", result.ShellSessionID, failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeInternal,
			ExitCode:   poc.ExitCodeInternal,
			Facts:      []poc.Fact{{Message: "export report: " + err.Error()}},
			Suggestions: []poc.Suggestion{
				{Message: "check --report path and retry"},
			},
		})
	}

	attachCtx, cancelAttach := context.WithTimeout(ctx, 10*time.Second)
	defer cancelAttach()

	stream, err := client.DialShell(attachCtx, result.ShellSessionID)
	if err != nil {
		return exitWithError(opt, stdout, stderr, "sh", result.ShellSessionID, err)
	}
	defer func() { _ = stream.Close() }()

	stdinFD := deps.stdinFD()
	if deps.isTerminal(stdinFD) {
		oldState, err := deps.makeRaw(stdinFD)
		if err == nil {
			defer func() { _ = deps.restoreTerm(stdinFD, oldState) }()
		}
	}

	writeCh := make(chan shellFrame, 64)
	var writerWG sync.WaitGroup

	writerWG.Add(1)
	go func() {
		defer writerWG.Done()
		defer cancel()

		for {
			select {
			case <-ctx.Done():
				return
			case frame := <-writeCh:
				switch frame.kind {
				case "json":
					if err := shellproto.WriteFrame(stream, shellproto.KindJSON, frame.payload); err != nil {
						return
					}
				default:
					if err := shellproto.WriteFrame(stream, shellproto.KindData, frame.payload); err != nil {
						return
					}
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
		case writeCh <- shellFrame{kind: "json", payload: payload}:
		case <-ctx.Done():
		}
	}

	if cols, rows, err := deps.getSize(stdinFD); err == nil {
		sendWinSize(cols, rows)
	}
	deps.watchResize(ctx, stdinFD, sendWinSize)

	go func() {
		defer cancel()

		buf := make([]byte, 32*1024)
		for {
			n, err := deps.stdin.Read(buf)
			if n > 0 {
				payload := append([]byte(nil), buf[:n]...)
				select {
				case writeCh <- shellFrame{kind: "data", payload: payload}:
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
		kind, payload, err := shellproto.ReadFrame(stream)
		if err != nil {
			if errors.Is(err, io.EOF) {
				break
			}
			cancel()
			writerWG.Wait()
			return exitWithFailure(opt, stdout, stderr, "sh", result.ShellSessionID, failureOutput{
				Stage:      "Shell",
				ReasonCode: poc.ReasonCodeUnavailable,
				ExitCode:   poc.ExitCodeUnavailable,
				Facts:      []poc.Fact{{Message: "shell stream failed: " + err.Error()}},
				Suggestions: []poc.Suggestion{
					{Message: "retry"},
				},
			})
		}

		switch kind {
		case shellproto.KindData:
			_, _ = stdout.Write(payload)
		case shellproto.KindJSON:
			var control shellproto.Control
			if err := json.Unmarshal(payload, &control); err != nil {
				cancel()
				writerWG.Wait()
				return exitWithFailure(opt, stdout, stderr, "sh", result.ShellSessionID, failureOutput{
					Stage:      "Shell",
					ReasonCode: poc.ReasonCodeInternal,
					ExitCode:   poc.ExitCodeInternal,
					Facts:      []poc.Fact{{Message: "decode shell control: " + err.Error()}},
					Suggestions: []poc.Suggestion{
						{Message: "retry"},
					},
				})
			}
			if control.Op == shellproto.OpShellExit {
				cancel()
				writerWG.Wait()
				if control.OK {
					return 0
				}
				return exitWithFailure(opt, stdout, stderr, "sh", result.ShellSessionID, failureOutput{
					Stage:      "Shell",
					ReasonCode: poc.ReasonCodeUnavailable,
					ExitCode:   poc.ExitCodeUnavailable,
					Facts:      []poc.Fact{{Message: "remote shell exited with failure"}},
					Suggestions: []poc.Suggestion{
						{Message: "retry"},
					},
				})
			}
		}
	}

	cancel()
	writerWG.Wait()
	return 0
}
