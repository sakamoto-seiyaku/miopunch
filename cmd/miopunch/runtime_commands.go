package main

import (
	"context"
	"io"
	"strconv"
	"strings"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
)

func runLS(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	if len(args) != 0 {
		return exitWithFailure(opt, stdout, stderr, "ls", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts:      []poc.Fact{{Message: "unexpected extra args"}},
			Suggestions: []poc.Suggestion{
				{Message: "use: miopunch ls"},
			},
		})
	}
	return runRuntimeAction(opt, "ls", "ls", nil, stdout, stderr)
}

func runInitNetwork(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	request := pocruntime.InitNetworkArgs{}
	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch {
		case arg == "--new":
			request.CreateNew = true
		case arg == "--confirm":
			if i+1 >= len(args) {
				return exitWithFailure(opt, stdout, stderr, "init-network", "", failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "missing value for --confirm"}},
					Suggestions: []poc.Suggestion{
						{Message: "use: miopunch init-network --new --confirm create-new-network"},
					},
				})
			}
			i++
			request.Confirm = strings.TrimSpace(args[i])
		case strings.HasPrefix(arg, "--confirm="):
			request.Confirm = strings.TrimSpace(strings.TrimPrefix(arg, "--confirm="))
		default:
			return exitWithFailure(opt, stdout, stderr, "init-network", "", failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Facts:      []poc.Fact{{Message: "unknown arg: " + arg}},
				Suggestions: []poc.Suggestion{
					{Message: "use: miopunch init-network [--new --confirm create-new-network]"},
				},
			})
		}
	}
	return runRuntimeAction(opt, "init-network", "init-network", request, stdout, stderr)
}

func runInvite(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	request := pocruntime.InviteArgs{}
	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch {
		case arg == "--mode":
			if i+1 >= len(args) {
				return missingFlagValueFailure(opt, stdout, stderr, "invite", "--mode", "use: miopunch invite --mode approve|auto")
			}
			i++
			request.Mode = strings.TrimSpace(args[i])
		case strings.HasPrefix(arg, "--mode="):
			request.Mode = strings.TrimSpace(strings.TrimPrefix(arg, "--mode="))
		case arg == "--uses":
			if i+1 >= len(args) {
				return missingFlagValueFailure(opt, stdout, stderr, "invite", "--uses", "use: miopunch invite --uses 1")
			}
			i++
			n, err := strconv.Atoi(strings.TrimSpace(args[i]))
			if err != nil || n <= 0 {
				return invalidArgFailure(opt, stdout, stderr, "invite", "invalid --uses", "use: miopunch invite --uses 1")
			}
			request.MaxUses = n
		case strings.HasPrefix(arg, "--uses="):
			n, err := strconv.Atoi(strings.TrimSpace(strings.TrimPrefix(arg, "--uses=")))
			if err != nil || n <= 0 {
				return invalidArgFailure(opt, stdout, stderr, "invite", "invalid --uses", "use: miopunch invite --uses 1")
			}
			request.MaxUses = n
		case arg == "--expires":
			if i+1 >= len(args) {
				return missingFlagValueFailure(opt, stdout, stderr, "invite", "--expires", "use: miopunch invite --expires 15m")
			}
			i++
			request.Expires = strings.TrimSpace(args[i])
		case strings.HasPrefix(arg, "--expires="):
			request.Expires = strings.TrimSpace(strings.TrimPrefix(arg, "--expires="))
		default:
			return exitWithFailure(opt, stdout, stderr, "invite", "", failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Facts:      []poc.Fact{{Message: "unknown flag: " + arg}},
				Suggestions: []poc.Suggestion{
					{Message: "use: miopunch invite [--mode ...] [--uses ...] [--expires ...]"},
				},
			})
		}
	}
	return runRuntimeAction(opt, "invite", "invite", request, stdout, stderr)
}

func runApprove(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	if len(args) < 1 || strings.TrimSpace(args[0]) == "" {
		return exitWithFailure(opt, stdout, stderr, "approve", "", failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts:      []poc.Fact{{Message: "missing invite code"}},
			Suggestions: []poc.Suggestion{
				{Message: "use: miopunch approve <invite_code>"},
			},
		})
	}
	return runRuntimeAction(opt, "approve", "approve", pocruntime.ApproveArgs{Code: args[0]}, stdout, stderr)
}

func runJoin(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	request := pocruntime.JoinArgs{}
	if len(args) >= 1 {
		request.Code = args[0]
	}
	return runRuntimeAction(opt, "join", "join", request, stdout, stderr)
}

func runPing(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	peerID, p2pNetwork, failure := parsePeerAndP2PArgs(opt, "ping", args, "use: miopunch ping <peer_id> [-u|-t|--p2p-network ...]")
	if failure != nil {
		return exitWithFailure(opt, stdout, stderr, "ping", "", *failure)
	}
	return runRuntimeAction(opt, "ping", "ping", pocruntime.PingArgs{
		PeerID:     peerID,
		P2PNetwork: p2pNetwork,
	}, stdout, stderr)
}

func runShLS(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	peerID, target, p2pNetwork, failure := parseShellArgs(opt, "sh ls", args, "use: miopunch sh ls <peer_id> [target] [-u|-t|--p2p-network ...]")
	if failure != nil {
		return exitWithFailure(opt, stdout, stderr, "sh ls", "", *failure)
	}
	return runRuntimeAction(opt, "sh ls", "sh_ls", pocruntime.ShellArgs{
		PeerID:     peerID,
		Target:     target,
		P2PNetwork: p2pNetwork,
	}, stdout, stderr)
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
	dangerous := false
	for _, arg := range args[1:] {
		switch arg {
		case "--dangerous", "--dangerous=true":
			dangerous = true
		default:
			return exitWithFailure(opt, stdout, stderr, "revoke", "", failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Facts:      []poc.Fact{{Message: "unknown arg: " + arg}},
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
			Facts:      []poc.Fact{{Message: "missing --dangerous (revoke is irreversible in the POC runtime)"}},
			Suggestions: []poc.Suggestion{
				{Message: "re-run with: miopunch revoke <peer_id> --dangerous"},
			},
		})
	}
	return runRuntimeAction(opt, "revoke", "revoke", pocruntime.RevokeArgs{
		PeerID:    args[0],
		Dangerous: true,
	}, stdout, stderr)
}

func runRuntimeAction(
	opt globalOptions,
	kind string,
	action string,
	args any,
	stdout io.Writer,
	stderr io.Writer,
) int {
	ctx, cancel := context.WithTimeout(context.Background(), actionTimeout(action))
	defer cancel()

	client, _, err := connectLocalAPI(ctx, opt.LocalAPIOverride)
	if err != nil {
		return exitWithError(opt, stdout, stderr, kind, "", err)
	}
	result, err := client.Action(ctx, action, args)
	if err != nil {
		return exitWithError(opt, stdout, stderr, kind, "", err)
	}
	return exitWithActionSuccess(opt, stdout, stderr, kind, result)
}

func actionTimeout(action string) time.Duration {
	switch action {
	case "approve", "join":
		return 3 * time.Minute
	case "sh":
		return 2 * time.Minute
	default:
		return 30 * time.Second
	}
}

func missingFlagValueFailure(
	opt globalOptions,
	stdout io.Writer,
	stderr io.Writer,
	kind string,
	flag string,
	usage string,
) int {
	return exitWithFailure(opt, stdout, stderr, kind, "", failureOutput{
		Stage:      "cli",
		ReasonCode: poc.ReasonCodeBadRequest,
		ExitCode:   poc.ExitCodeBadRequest,
		Facts:      []poc.Fact{{Message: "missing value for " + flag}},
		Suggestions: []poc.Suggestion{
			{Message: usage},
		},
	})
}

func invalidArgFailure(
	opt globalOptions,
	stdout io.Writer,
	stderr io.Writer,
	kind string,
	message string,
	usage string,
) int {
	return exitWithFailure(opt, stdout, stderr, kind, "", failureOutput{
		Stage:      "cli",
		ReasonCode: poc.ReasonCodeBadRequest,
		ExitCode:   poc.ExitCodeBadRequest,
		Facts:      []poc.Fact{{Message: message}},
		Suggestions: []poc.Suggestion{
			{Message: usage},
		},
	})
}

func parsePeerAndP2PArgs(
	opt globalOptions,
	kind string,
	args []string,
	usage string,
) (string, string, *failureOutput) {
	peerID := ""
	p2pNetwork := "auto"

	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch {
		case arg == "-u":
			p2pNetwork = "udp_only"
		case arg == "-t":
			p2pNetwork = "tcp_only"
		case arg == "--p2p-network":
			if i+1 >= len(args) {
				failure := failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "missing value for --p2p-network"}},
					Suggestions: []poc.Suggestion{
						{Message: usage},
					},
				}
				return "", "", &failure
			}
			i++
			p2pNetwork = args[i]
		case strings.HasPrefix(arg, "--p2p-network="):
			p2pNetwork = strings.TrimPrefix(arg, "--p2p-network=")
		case strings.HasPrefix(arg, "-"):
			failure := failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Facts:      []poc.Fact{{Message: "unknown arg: " + arg}},
				Suggestions: []poc.Suggestion{
					{Message: usage},
				},
			}
			return "", "", &failure
		default:
			if peerID == "" {
				peerID = arg
				continue
			}
			failure := failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Facts:      []poc.Fact{{Message: "unexpected extra arg: " + arg}},
				Suggestions: []poc.Suggestion{
					{Message: usage},
				},
			}
			return "", "", &failure
		}
	}

	network, err := connectivity.ParseP2PNetwork(p2pNetwork)
	if err != nil {
		failure := failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts:      []poc.Fact{{Message: err.Error()}},
			Suggestions: []poc.Suggestion{
				{Message: usage},
			},
		}
		return "", "", &failure
	}
	return peerID, string(network), nil
}

func parseShellArgs(
	opt globalOptions,
	kind string,
	args []string,
	usage string,
) (string, string, string, *failureOutput) {
	_ = opt
	_ = kind
	peerID := ""
	target := ""
	p2pNetwork := "auto"

	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch {
		case arg == "-u":
			p2pNetwork = "udp_only"
		case arg == "-t":
			p2pNetwork = "tcp_only"
		case arg == "--p2p-network":
			if i+1 >= len(args) {
				failure := failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "missing value for --p2p-network"}},
					Suggestions: []poc.Suggestion{
						{Message: usage},
					},
				}
				return "", "", "", &failure
			}
			i++
			p2pNetwork = args[i]
		case strings.HasPrefix(arg, "--p2p-network="):
			p2pNetwork = strings.TrimPrefix(arg, "--p2p-network=")
		case strings.HasPrefix(arg, "-"):
			failure := failureOutput{
				Stage:      "cli",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Facts:      []poc.Fact{{Message: "unknown arg: " + arg}},
				Suggestions: []poc.Suggestion{
					{Message: usage},
				},
			}
			return "", "", "", &failure
		default:
			if peerID == "" {
				peerID = arg
			} else if target == "" {
				target = arg
			} else {
				failure := failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "unexpected extra arg: " + arg}},
					Suggestions: []poc.Suggestion{
						{Message: usage},
					},
				}
				return "", "", "", &failure
			}
		}
	}

	network, err := connectivity.ParseP2PNetwork(p2pNetwork)
	if err != nil {
		failure := failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts:      []poc.Fact{{Message: err.Error()}},
			Suggestions: []poc.Suggestion{
				{Message: usage},
			},
		}
		return "", "", "", &failure
	}
	return peerID, target, string(network), nil
}
