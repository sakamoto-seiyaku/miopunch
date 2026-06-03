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
	usage := "use: miopunch ping <peer_id> [-u|-t] [-4|-6] [--p2p-network auto|udp_only|tcp_only] [--p2p-ip-family auto|v4|v6]"
	peerID, p2pNetwork, p2pIPFamily, failure := parsePeerAndP2PArgs(opt, "ping", args, usage)
	if failure != nil {
		return exitWithFailure(opt, stdout, stderr, "ping", "", *failure)
	}
	return runRuntimeAction(opt, "ping", "ping", pocruntime.PingArgs{
		PeerID:      peerID,
		P2PNetwork:  p2pNetwork,
		P2PIPFamily: p2pIPFamily,
	}, stdout, stderr)
}

func runShLS(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	usage := "use: miopunch sh ls <peer_id> [target] [--ready] [-u|-t] [-4|-6] [--p2p-network auto|udp_only|tcp_only] [--p2p-ip-family auto|v4|v6]"
	peerID, target, p2pNetwork, p2pIPFamily, readyOnly, failure := parseShellArgs(
		opt,
		"sh ls",
		args,
		usage,
	)
	if failure != nil {
		return exitWithFailure(opt, stdout, stderr, "sh ls", "", *failure)
	}
	return runRuntimeAction(opt, "sh ls", "sh_ls", pocruntime.ShellArgs{
		PeerID:      peerID,
		Target:      target,
		P2PNetwork:  p2pNetwork,
		P2PIPFamily: p2pIPFamily,
		ReadyOnly:   readyOnly,
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
) (string, string, string, *failureOutput) {
	peerID := ""
	p2pNetwork := "auto"
	p2pIPFamily := "auto"
	var networkSet bool
	var ipFamilySet bool

	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch {
		case arg == "-u":
			if networkSet && p2pNetwork != "udp_only" {
				return "", "", "", badArgFailure("conflicting p2p network flags", usage)
			}
			p2pNetwork = "udp_only"
			networkSet = true
		case arg == "-t":
			if networkSet && p2pNetwork != "tcp_only" {
				return "", "", "", badArgFailure("conflicting p2p network flags", usage)
			}
			p2pNetwork = "tcp_only"
			networkSet = true
		case arg == "-4":
			if ipFamilySet && p2pIPFamily != "v4" {
				return "", "", "", badArgFailure("conflicting p2p ip family flags", usage)
			}
			p2pIPFamily = "v4"
			ipFamilySet = true
		case arg == "-6":
			if ipFamilySet && p2pIPFamily != "v6" {
				return "", "", "", badArgFailure("conflicting p2p ip family flags", usage)
			}
			p2pIPFamily = "v6"
			ipFamilySet = true
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
			network, err := connectivity.ParseP2PNetwork(args[i])
			if err != nil {
				return "", "", "", badArgFailure(err.Error(), usage)
			}
			if networkSet && p2pNetwork != string(network) && network != connectivity.P2PNetworkAuto {
				return "", "", "", badArgFailure("conflicting p2p network flags", usage)
			}
			p2pNetwork = string(network)
			if network != connectivity.P2PNetworkAuto {
				networkSet = true
			}
		case strings.HasPrefix(arg, "--p2p-network="):
			network, err := connectivity.ParseP2PNetwork(strings.TrimPrefix(arg, "--p2p-network="))
			if err != nil {
				return "", "", "", badArgFailure(err.Error(), usage)
			}
			if networkSet && p2pNetwork != string(network) && network != connectivity.P2PNetworkAuto {
				return "", "", "", badArgFailure("conflicting p2p network flags", usage)
			}
			p2pNetwork = string(network)
			if network != connectivity.P2PNetworkAuto {
				networkSet = true
			}
		case arg == "--p2p-ip-family":
			if i+1 >= len(args) {
				failure := failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "missing value for --p2p-ip-family"}},
					Suggestions: []poc.Suggestion{
						{Message: usage},
					},
				}
				return "", "", "", &failure
			}
			i++
			family, err := connectivity.ParseP2PIPFamily(args[i])
			if err != nil {
				return "", "", "", badArgFailure(err.Error(), usage)
			}
			if ipFamilySet && p2pIPFamily != string(family) && family != connectivity.P2PIPFamilyAuto {
				return "", "", "", badArgFailure("conflicting p2p ip family flags", usage)
			}
			p2pIPFamily = string(family)
			if family != connectivity.P2PIPFamilyAuto {
				ipFamilySet = true
			}
		case strings.HasPrefix(arg, "--p2p-ip-family="):
			family, err := connectivity.ParseP2PIPFamily(strings.TrimPrefix(arg, "--p2p-ip-family="))
			if err != nil {
				return "", "", "", badArgFailure(err.Error(), usage)
			}
			if ipFamilySet && p2pIPFamily != string(family) && family != connectivity.P2PIPFamilyAuto {
				return "", "", "", badArgFailure("conflicting p2p ip family flags", usage)
			}
			p2pIPFamily = string(family)
			if family != connectivity.P2PIPFamilyAuto {
				ipFamilySet = true
			}
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
			return "", "", "", &failure
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
	family, err := connectivity.ParseP2PIPFamily(p2pIPFamily)
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
	return peerID, string(network), string(family), nil
}

func parseShellArgs(
	opt globalOptions,
	kind string,
	args []string,
	usage string,
) (string, string, string, string, bool, *failureOutput) {
	_ = opt
	_ = kind
	peerID := ""
	target := ""
	p2pNetwork := "auto"
	p2pIPFamily := "auto"
	readyOnly := false
	var networkSet bool
	var ipFamilySet bool

	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch {
		case arg == "--ready":
			readyOnly = true
		case arg == "-u":
			if networkSet && p2pNetwork != "udp_only" {
				return "", "", "", "", false, badArgFailure("conflicting p2p network flags", usage)
			}
			p2pNetwork = "udp_only"
			networkSet = true
		case arg == "-t":
			if networkSet && p2pNetwork != "tcp_only" {
				return "", "", "", "", false, badArgFailure("conflicting p2p network flags", usage)
			}
			p2pNetwork = "tcp_only"
			networkSet = true
		case arg == "-4":
			if ipFamilySet && p2pIPFamily != "v4" {
				return "", "", "", "", false, badArgFailure("conflicting p2p ip family flags", usage)
			}
			p2pIPFamily = "v4"
			ipFamilySet = true
		case arg == "-6":
			if ipFamilySet && p2pIPFamily != "v6" {
				return "", "", "", "", false, badArgFailure("conflicting p2p ip family flags", usage)
			}
			p2pIPFamily = "v6"
			ipFamilySet = true
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
				return "", "", "", "", false, &failure
			}
			i++
			network, err := connectivity.ParseP2PNetwork(args[i])
			if err != nil {
				return "", "", "", "", false, badArgFailure(err.Error(), usage)
			}
			if networkSet && p2pNetwork != string(network) && network != connectivity.P2PNetworkAuto {
				return "", "", "", "", false, badArgFailure("conflicting p2p network flags", usage)
			}
			p2pNetwork = string(network)
			if network != connectivity.P2PNetworkAuto {
				networkSet = true
			}
		case strings.HasPrefix(arg, "--p2p-network="):
			network, err := connectivity.ParseP2PNetwork(strings.TrimPrefix(arg, "--p2p-network="))
			if err != nil {
				return "", "", "", "", false, badArgFailure(err.Error(), usage)
			}
			if networkSet && p2pNetwork != string(network) && network != connectivity.P2PNetworkAuto {
				return "", "", "", "", false, badArgFailure("conflicting p2p network flags", usage)
			}
			p2pNetwork = string(network)
			if network != connectivity.P2PNetworkAuto {
				networkSet = true
			}
		case arg == "--p2p-ip-family":
			if i+1 >= len(args) {
				failure := failureOutput{
					Stage:      "cli",
					ReasonCode: poc.ReasonCodeBadRequest,
					ExitCode:   poc.ExitCodeBadRequest,
					Facts:      []poc.Fact{{Message: "missing value for --p2p-ip-family"}},
					Suggestions: []poc.Suggestion{
						{Message: usage},
					},
				}
				return "", "", "", "", false, &failure
			}
			i++
			family, err := connectivity.ParseP2PIPFamily(args[i])
			if err != nil {
				return "", "", "", "", false, badArgFailure(err.Error(), usage)
			}
			if ipFamilySet && p2pIPFamily != string(family) && family != connectivity.P2PIPFamilyAuto {
				return "", "", "", "", false, badArgFailure("conflicting p2p ip family flags", usage)
			}
			p2pIPFamily = string(family)
			if family != connectivity.P2PIPFamilyAuto {
				ipFamilySet = true
			}
		case strings.HasPrefix(arg, "--p2p-ip-family="):
			family, err := connectivity.ParseP2PIPFamily(strings.TrimPrefix(arg, "--p2p-ip-family="))
			if err != nil {
				return "", "", "", "", false, badArgFailure(err.Error(), usage)
			}
			if ipFamilySet && p2pIPFamily != string(family) && family != connectivity.P2PIPFamilyAuto {
				return "", "", "", "", false, badArgFailure("conflicting p2p ip family flags", usage)
			}
			p2pIPFamily = string(family)
			if family != connectivity.P2PIPFamilyAuto {
				ipFamilySet = true
			}
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
			return "", "", "", "", false, &failure
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
				return "", "", "", "", false, &failure
			}
		}
	}
	if readyOnly && strings.TrimSpace(target) != "" {
		failure := failureOutput{
			Stage:      "cli",
			ReasonCode: poc.ReasonCodeBadRequest,
			ExitCode:   poc.ExitCodeBadRequest,
			Facts:      []poc.Fact{{Message: "--ready cannot be combined with a concrete target"}},
			Suggestions: []poc.Suggestion{
				{Message: "use: miopunch sh ls <peer_id> --ready"},
				{Message: "or: miopunch sh ls <peer_id> <target>"},
			},
		}
		return "", "", "", "", false, &failure
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
		return "", "", "", "", false, &failure
	}
	family, err := connectivity.ParseP2PIPFamily(p2pIPFamily)
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
		return "", "", "", "", false, &failure
	}
	return peerID, target, string(network), string(family), readyOnly, nil
}

func badArgFailure(message string, usage string) *failureOutput {
	return &failureOutput{
		Stage:      "cli",
		ReasonCode: poc.ReasonCodeBadRequest,
		ExitCode:   poc.ExitCodeBadRequest,
		Facts:      []poc.Fact{{Message: message}},
		Suggestions: []poc.Suggestion{
			{Message: usage},
		},
	}
}
