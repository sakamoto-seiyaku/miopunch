#!/usr/bin/env bash
# Reproduce MIOPUNCH-28's public create/stop/restart entry on Linux.
set -euo pipefail

binary="${1:?pass path to miopunch-native-probe binary}"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
node_dir="$scratch/node"

created="$("$binary" network create "$node_dir")"
first_restart="$("$binary" network status "$node_dir")"
[[ "$created" == "$first_restart" ]]

# SIGINT asks the running endpoint to close cleanly, as Ctrl-C does at a terminal.
run_result="$(timeout --preserve-status --signal=INT --kill-after=2s 3s "$binary" network run "$node_dir" 127.0.0.1)"
[[ "$run_result" == READY* ]]
second_restart="$("$binary" network status "$node_dir")"
[[ "$created" == "$second_restart" ]]

printf '%s\n' "$created" "$run_result" 'RESTART_OK'
