package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"time"

	"github.com/miopunch/miopunch/internal/poc"
)

func runTopology(opt globalOptions, args []string, stdout, stderr io.Writer) int {
	_ = args

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	c, _, err := connectLocalAPI(ctx, opt.LocalAPIOverride)
	if err != nil {
		return exitWithError(opt, stdout, stderr, "topology", "", err)
	}

	snap, err := c.GetTopology(ctx)
	if err != nil {
		return exitWithError(opt, stdout, stderr, "topology", "", err)
	}

	if opt.Format == outputFormatJSON {
		enc := json.NewEncoder(stdout)
		_ = enc.Encode(snap)
		return 0
	}

	memberCount := len(snap.Members)
	activeCount := len(snap.Neighbors.Active)
	netID := snap.Net.NetID
	if netID == "" {
		netID = "-"
	}

	fmt.Fprintf(stdout, "peer_id=%s\n", snap.Self.PeerID)
	fmt.Fprintf(stdout, "role=%s\n", snap.Self.Role)
	fmt.Fprintf(stdout, "net_id=%s\n", netID)
	fmt.Fprintf(stdout, "member_count=%d\n", memberCount)
	fmt.Fprintf(stdout, "active_neighbors=%d\n", activeCount)
	if snap.Self.V4Hint != "" {
		fmt.Fprintf(stdout, "v4_hint=%s\n", snap.Self.V4Hint)
	}
	if snap.Self.V6Hint != "" {
		fmt.Fprintf(stdout, "v6_hint=%s\n", snap.Self.V6Hint)
	}
	return int(poc.ExitCodeOK)
}
