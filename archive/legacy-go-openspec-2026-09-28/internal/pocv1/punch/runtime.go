// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

package punch

import (
	"context"
	"crypto/sha256"
	"encoding/base32"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"strings"
	"sync"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/punchdecision"
	"github.com/miopunch/miopunch/internal/punching"
	"github.com/miopunch/miopunch/internal/udpowner"
	legacywire "github.com/miopunch/miopunch/internal/wire"
)

func runPunch(
	ctx context.Context,
	cfg LoadedConfig,
	remote trustedRemote,
	dialID string,
	punchToken []byte,
	resp *legacywire.NatHoleResp,
	decision UDPDecision,
	initiator bool,
) (PathResult, error) {
	if resp == nil {
		return PathResult{}, wrapDiagnosticError(Diagnostic{
			DialID:             dialID,
			RemotePeerID:       remote.PeerID,
			LocalCandidates:    cfg.LocalCandidates,
			AttemptConcurrency: cfg.AttemptConcurrency,
			AttemptBudget:      cfg.AttemptBudget,
		}, errors.New("missing udp decision response"))
	}
	logutil.Debugf(
		"punch run start: dial_id=%s remote_peer_id=%s local_candidates=%d mode=%d role=%s attempt_budget_ms=%d initiator=%t p2p_network=%s p2p_ip_family=%s",
		dialID,
		remote.PeerID,
		len(cfg.LocalCandidates),
		resp.DetectBehavior.Mode,
		resp.DetectBehavior.Role,
		cfg.AttemptBudget.Milliseconds(),
		initiator,
		cfg.P2PNetwork,
		cfg.P2PIPFamily,
	)
	logutil.Debugf(
		"punch run response input: dial_id=%s remote_peer_id=%s sid=%s peer_direct_addrs=%v candidate_addrs=%v assisted_addrs=%v punching_enabled=%t punching_error=%q detect_behavior=%+v p2p_network=%s",
		dialID,
		remote.PeerID,
		resp.Sid,
		resp.PeerDirectAddrs,
		resp.CandidateAddrs,
		resp.AssistedAddrs,
		resp.PunchingEnabled,
		resp.PunchingError,
		resp.DetectBehavior,
		resp.P2PNetwork,
	)
	attemptCtx, cancel := withAttemptBudget(ctx, cfg.AttemptBudget)
	defer cancel()
	attemptUDP := cfg.AttemptUDP
	if attemptUDP == nil {
		attemptUDP = defaultUDPAttemptForPolicy(cfg.P2PNetwork, cfg.P2PIPFamily)
	}

	udp4Demux, err := openTraversalDemux(cfg.UDPConn, cfg.UDPOwner, punchToken)
	if err != nil {
		return PathResult{}, wrapDiagnosticError(Diagnostic{
			DialID:             dialID,
			RemotePeerID:       remote.PeerID,
			LocalCandidates:    cfg.LocalCandidates,
			AttemptConcurrency: cfg.AttemptConcurrency,
			AttemptBudget:      cfg.AttemptBudget,
		}, fmt.Errorf("open udp4 traversal demux: %w", err))
	}
	defer udp4Demux.Close()

	var udp6Demux *udpowner.TraversalDemux
	if cfg.UDP6Conn != nil {
		udp6Demux, err = openTraversalDemux(cfg.UDP6Conn, cfg.UDP6Owner, punchToken)
		if err != nil {
			return PathResult{}, wrapDiagnosticError(Diagnostic{
				DialID:             dialID,
				RemotePeerID:       remote.PeerID,
				LocalCandidates:    cfg.LocalCandidates,
				AttemptConcurrency: cfg.AttemptConcurrency,
				AttemptBudget:      cfg.AttemptBudget,
			}, fmt.Errorf("open udp6 traversal demux: %w", err))
		}
		defer udp6Demux.Close()
	}

	attemptRes, err := attemptUDP(attemptCtx, dialID, punchToken, cfg.UDPConn, cfg.UDP6Conn, resp, udp4Demux, udp6Demux)
	if err != nil {
		logutil.Warnf(
			"punch run failed: dial_id=%s remote_peer_id=%s err=%v",
			dialID,
			remote.PeerID,
			err,
		)
		return PathResult{}, wrapDiagnosticError(Diagnostic{
			DialID:             dialID,
			RemotePeerID:       remote.PeerID,
			LocalCandidates:    cfg.LocalCandidates,
			AttemptConcurrency: cfg.AttemptConcurrency,
			AttemptBudget:      cfg.AttemptBudget,
		}, err)
	}
	if attemptRes == nil || attemptRes.Conn == nil || attemptRes.Remote == nil {
		return PathResult{}, errors.New("udp attempt selected incomplete result")
	}
	ownership, packetConn := selectedUDPOwnership(cfg, attemptRes.Conn)
	if attemptRes.Path == PathPunchingIPv4 {
		punchdecision.ReportDaemonUDPSuccess(&punchdecision.Result{
			AnalyzerKey: localUDPAnalyzerKey(remote.PeerID, decision),
			Mode:        decision.Mode,
			Index:       decision.Index,
		})
	}
	selectedLocal := firstCandidate(cfg.LocalCandidates)
	selectedRemote := candidateForRemote(resp, attemptRes.Remote)
	allowedRemoteUDPAddrs := allowedRemoteUDPAddrsForAttempt(resp, attemptRes)
	attemptedPairs := attemptEvidenceFromConnectivity(cfg.LocalCandidates, resp, attemptRes)
	if len(attemptedPairs) == 0 {
		attemptedPairs = []AttemptEvidence{{
			LocalCandidate:  selectedLocal,
			RemoteCandidate: selectedRemote,
			Path:            attemptRes.Path,
			Result:          "selected",
			Detail:          attemptRes.Remote.String(),
		}}
	}
	logutil.Debugf(
		"punch run attempt result: dial_id=%s remote_peer_id=%s path=%s ownership=%s conn_local_addr=%s remote_addr=%s allowed_remote_udp_addrs=%v selected_local=%+v selected_remote=%+v attempted_pairs=%+v",
		dialID,
		remote.PeerID,
		attemptRes.Path,
		ownership,
		udpConnAddrString(attemptRes.Conn),
		attemptRes.Remote.String(),
		allowedRemoteUDPAddrs,
		selectedLocal,
		selectedRemote,
		attemptedPairs,
	)
	logutil.Infof(
		"punch run selected: dial_id=%s remote_peer_id=%s selected_path=%s ownership=%s local_candidate=%s remote_candidate=%s remote_udp=%s allowed_remote_udp_addrs=%v attempted_pairs=%s",
		dialID,
		remote.PeerID,
		attemptRes.Path,
		ownership,
		selectedLocal.Addr,
		selectedRemote.Addr,
		attemptRes.Remote.String(),
		allowedRemoteUDPAddrs,
		formatAttemptEvidence(attemptedPairs),
	)

	return PathResult{
		Conn:                  attemptRes.Conn,
		RemoteAddr:            attemptRes.Remote,
		AllowedRemoteUDPAddrs: allowedRemoteUDPAddrs,
		UDPOwnership:          ownership,
		RuntimeKCPPacket:      packetConn,
		RemoteIdentity: TrustedRemoteIdentity{
			PeerID:           remote.PeerID,
			MemberCredential: append([]byte(nil), remote.MemberCredential...),
		},
		Evidence: PunchEvidence{
			DialID:            dialID,
			AttemptedPairs:    attemptedPairs,
			SelectedPath:      attemptRes.Path,
			SelectedLocal:     selectedLocal,
			SelectedRemote:    selectedRemote,
			SelectedRemoteUDP: attemptRes.Remote.String(),
		},
	}, nil
}

func localUDPAnalyzerKey(remotePeerID string, decision UDPDecision) string {
	if strings.TrimSpace(decision.AnalysisKey) != "" {
		return punchdecision.UDPAnalyzerKey(remotePeerID, decision.AnalysisKey)
	}
	return decision.AnalyzerKey
}

func openTraversalDemux(conn *net.UDPConn, owner *udpowner.KCPOwner, key []byte) (*udpowner.TraversalDemux, error) {
	if owner != nil {
		return owner.OpenTraversalDemux(udpowner.DemuxConfig{Key: key})
	}
	return udpowner.NewUDPTraversalDemux(conn, udpowner.DemuxConfig{Key: key})
}

func selectedUDPOwnership(cfg LoadedConfig, conn *net.UDPConn) (SelectedUDPOwnership, net.PacketConn) {
	switch conn {
	case cfg.UDPConn:
		if cfg.UDPOwner != nil {
			return SelectedUDPOwnershipRuntime, cfg.UDPOwner.BorrowPacketConn()
		}
		return SelectedUDPOwnershipRuntime, nil
	case cfg.UDP6Conn:
		if cfg.UDP6Owner != nil {
			return SelectedUDPOwnershipRuntime, cfg.UDP6Owner.BorrowPacketConn()
		}
		return SelectedUDPOwnershipRuntime, nil
	default:
		return SelectedUDPOwnershipTemporary, nil
	}
}

func allowedRemoteUDPAddrsForAttempt(resp *legacywire.NatHoleResp, attemptRes *connectivity.AttemptResult) []string {
	if attemptRes == nil || attemptRes.Remote == nil {
		return nil
	}

	observed := attemptRes.Remote.String()
	out := []string{observed}
	if attemptRes.Path != PathDirectIPv6 || resp == nil {
		return out
	}

	seen := map[string]struct{}{observed: {}}
	for _, raw := range resp.PeerDirectAddrs {
		addr := strings.TrimSpace(raw)
		if addr == "" {
			continue
		}
		ap, err := netip.ParseAddrPort(addr)
		if err != nil || !ap.Addr().Is6() {
			continue
		}
		if _, ok := seen[addr]; ok {
			continue
		}
		seen[addr] = struct{}{}
		out = append(out, addr)
	}
	return out
}

func udpConnAddrString(conn *net.UDPConn) string {
	if conn == nil || conn.LocalAddr() == nil {
		return ""
	}
	return conn.LocalAddr().String()
}

func formatAttemptEvidence(evidence []AttemptEvidence) string {
	if len(evidence) == 0 {
		return "[]"
	}
	parts := make([]string, 0, len(evidence))
	for _, ev := range evidence {
		parts = append(parts, fmt.Sprintf(
			"{path=%s result=%s local=%s remote=%s detail=%s}",
			ev.Path,
			ev.Result,
			ev.LocalCandidate.Addr,
			ev.RemoteCandidate.Addr,
			ev.Detail,
		))
	}
	return "[" + strings.Join(parts, ",") + "]"
}

func defaultUDPAttempt(
	ctx context.Context,
	sid string,
	key []byte,
	udp4Conn *net.UDPConn,
	udp6Conn *net.UDPConn,
	resp *legacywire.NatHoleResp,
	udp4Demux *udpowner.TraversalDemux,
	udp6Demux *udpowner.TraversalDemux,
) (*connectivity.AttemptResult, error) {
	ipFamily := connectivity.P2PIPFamilyV4
	if udp6Conn != nil {
		ipFamily = connectivity.P2PIPFamilyAuto
	}
	return defaultUDPAttemptForPolicy(connectivity.P2PNetworkUDPOnly, ipFamily)(
		ctx,
		sid,
		key,
		udp4Conn,
		udp6Conn,
		resp,
		udp4Demux,
		udp6Demux,
	)
}

func defaultUDPAttemptForPolicy(
	p2pNetwork connectivity.P2PNetwork,
	p2pIPFamily connectivity.P2PIPFamily,
) UDPAttemptFunc {
	return func(
		ctx context.Context,
		sid string,
		key []byte,
		udp4Conn *net.UDPConn,
		udp6Conn *net.UDPConn,
		resp *legacywire.NatHoleResp,
		udp4Demux *udpowner.TraversalDemux,
		udp6Demux *udpowner.TraversalDemux,
	) (*connectivity.AttemptResult, error) {
		return defaultUDPAttemptWithPolicy(
			ctx,
			sid,
			key,
			udp4Conn,
			udp6Conn,
			resp,
			udp4Demux,
			udp6Demux,
			p2pNetwork,
			p2pIPFamily,
		)
	}
}

func defaultUDPAttemptWithPolicy(
	ctx context.Context,
	sid string,
	key []byte,
	udp4Conn *net.UDPConn,
	udp6Conn *net.UDPConn,
	resp *legacywire.NatHoleResp,
	udp4Demux *udpowner.TraversalDemux,
	udp6Demux *udpowner.TraversalDemux,
	p2pNetwork connectivity.P2PNetwork,
	p2pIPFamily connectivity.P2PIPFamily,
) (*connectivity.AttemptResult, error) {
	return connectivity.Attempt(
		ctx,
		sid,
		key,
		udp4Conn,
		udp6Conn,
		nil,
		nil,
		resp,
		connectivity.AttemptConfig{
			P2PNetwork:         p2pNetwork,
			P2PIPFamily:        p2pIPFamily,
			UDP4TraversalDemux: udp4Demux,
			UDP6TraversalDemux: udp6Demux,
		},
	)
}

func firstCandidate(candidates []Candidate) Candidate {
	if len(candidates) == 0 {
		return Candidate{}
	}
	return candidates[0]
}

func candidateForRemote(resp *legacywire.NatHoleResp, remote *net.UDPAddr) Candidate {
	if remote == nil {
		return Candidate{}
	}
	return candidateForAddr(resp, remote.String())
}

func candidateForAddr(resp *legacywire.NatHoleResp, remoteAddr string) Candidate {
	remoteAddr = strings.TrimSpace(remoteAddr)
	if remoteAddr == "" {
		return Candidate{}
	}
	if resp != nil {
		for _, addr := range resp.PeerDirectAddrs {
			if strings.TrimSpace(addr) == remoteAddr {
				return Candidate{Kind: CandidateKindHost, Addr: remoteAddr}
			}
		}
		for _, addr := range resp.CandidateAddrs {
			if strings.TrimSpace(addr) == remoteAddr {
				return Candidate{Kind: CandidateKindSrflx, Addr: remoteAddr}
			}
		}
		for _, addr := range resp.AssistedAddrs {
			if strings.TrimSpace(addr) == remoteAddr {
				return Candidate{Kind: CandidateKindHost, Addr: remoteAddr}
			}
		}
	}
	return Candidate{Kind: CandidateKindSrflx, Addr: remoteAddr}
}

func attemptEvidenceFromConnectivity(localCandidates []Candidate, resp *legacywire.NatHoleResp, attemptRes *connectivity.AttemptResult) []AttemptEvidence {
	if attemptRes == nil || len(attemptRes.Evidence) == 0 {
		return nil
	}
	local := firstCandidate(localCandidates)
	out := make([]AttemptEvidence, 0, len(attemptRes.Evidence))
	for _, ev := range attemptRes.Evidence {
		out = append(out, AttemptEvidence{
			LocalCandidate:  local,
			RemoteCandidate: candidateForAddr(resp, ev.Candidate),
			Path:            ev.Path,
			Result:          ev.Result,
			Detail:          ev.Detail,
		})
	}
	return out
}

func buildPairPlans(
	conn *net.UDPConn,
	localCandidates []Candidate,
	remoteCandidates []Candidate,
	dialID string,
	initiator bool,
) ([]pairPlan, error) {
	remoteCandidates, err := normalizeCandidates(remoteCandidates)
	if err != nil {
		return nil, fmt.Errorf("%w: %w", ErrInvalidOffer, err)
	}
	plans := make([]pairPlan, 0, len(localCandidates)*len(remoteCandidates))
	for _, local := range localCandidates {
		for _, remote := range remoteCandidates {
			sid := sidForDialPair(dialID, local, remote)
			plans = append(plans, pairPlan{
				index:  len(plans),
				local:  local,
				remote: remote,
				sid:    sid,
				conn:   conn,
				resp:   natHoleRespForPair(remote, sid, initiator),
			})
		}
	}
	if len(plans) == 0 {
		return nil, ErrNoCandidatePairs
	}
	return plans, nil
}

func executePairPlans(
	ctx context.Context,
	cfg LoadedConfig,
	demux *udpowner.TraversalDemux,
	plans []pairPlan,
	key []byte,
) (SelectedAttempt, []AttemptEvidence, error) {
	evidence := make([]AttemptEvidence, len(plans))
	attemptCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	resultCh := make(chan SelectedAttempt, 1)
	errCh := make(chan error, len(plans))
	sem := make(chan struct{}, cfg.AttemptConcurrency)
	doneCh := make(chan struct{})
	var wg sync.WaitGroup

	for _, plan := range plans {
		plan := plan
		evidence[plan.index] = AttemptEvidence{
			LocalCandidate:  plan.local,
			RemoteCandidate: plan.remote,
			Result:          "pending",
		}
		wg.Add(1)
		go func() {
			defer wg.Done()
			logutil.Debugf(
				"punch attempt start: sid=%s local_candidate=%s remote_candidate=%s index=%d",
				plan.sid,
				plan.local.Addr,
				plan.remote.Addr,
				plan.index,
			)
			select {
			case sem <- struct{}{}:
			case <-attemptCtx.Done():
				err := attemptCtx.Err()
				evidence[plan.index].Result = attemptResultForError(err)
				evidence[plan.index].Detail = err.Error()
				logutil.Debugf(
					"punch attempt skipped: sid=%s local_candidate=%s remote_candidate=%s result=%s detail=%s",
					plan.sid,
					plan.local.Addr,
					plan.remote.Addr,
					evidence[plan.index].Result,
					evidence[plan.index].Detail,
				)
				errCh <- err
				return
			}
			defer func() { <-sem }()

			attemptResult, err := cfg.AttemptPair(attemptCtx, demux, plan, key)
			if err != nil {
				evidence[plan.index].Result = attemptResultForError(err)
				evidence[plan.index].Detail = err.Error()
				logutil.Debugf(
					"punch attempt failed: sid=%s local_candidate=%s remote_candidate=%s result=%s detail=%s",
					plan.sid,
					plan.local.Addr,
					plan.remote.Addr,
					evidence[plan.index].Result,
					evidence[plan.index].Detail,
				)
				errCh <- err
				return
			}
			attemptResult = normalizeAttemptPairResult(attemptResult)
			if attemptResult.RemoteAddr == nil {
				err := errors.New("attempt selected nil remote addr")
				evidence[plan.index].Result = "failed"
				evidence[plan.index].Detail = err.Error()
				errCh <- err
				return
			}

			evidence[plan.index].Path = attemptResult.Path
			evidence[plan.index].Result = "selected"
			evidence[plan.index].Detail = attemptResult.Detail
			logutil.Infof(
				"punch attempt selected: sid=%s local_candidate=%s remote_candidate=%s selected_path=%s remote_udp=%s",
				plan.sid,
				plan.local.Addr,
				plan.remote.Addr,
				attemptResult.Path,
				attemptResult.RemoteAddr.String(),
			)
			select {
			case resultCh <- SelectedAttempt{
				LocalCandidate:  plan.local,
				RemoteCandidate: plan.remote,
				Conn:            plan.conn,
				RemoteAddr:      attemptResult.RemoteAddr,
				Path:            attemptResult.Path,
			}:
			default:
			}
		}()
	}

	go func() {
		wg.Wait()
		close(doneCh)
		close(errCh)
	}()

	var firstErr error
	for {
		select {
		case selected := <-resultCh:
			cancel()
			<-doneCh
			return selected, evidence, nil
		case <-ctx.Done():
			cancel()
			<-doneCh
			if firstErr != nil {
				return SelectedAttempt{}, evidence, firstErr
			}
			return SelectedAttempt{}, evidence, ctx.Err()
		case err, ok := <-errCh:
			if !ok {
				if firstErr == nil {
					switch cause := context.Cause(ctx); {
					case errors.Is(cause, ErrAttemptBudgetExceeded):
						firstErr = ErrAttemptBudgetExceeded
					case cause != nil:
						firstErr = cause
					case errors.Is(ctx.Err(), context.DeadlineExceeded):
						firstErr = ErrAttemptBudgetExceeded
					case ctx.Err() != nil:
						firstErr = ctx.Err()
					default:
						firstErr = ErrAttemptBudgetExceeded
					}
				}
				logutil.Warnf(
					"punch attempts exhausted: planned_pairs=%d summary=%s err=%v",
					len(plans),
					summarizeAttemptResults(evidence),
					firstErr,
				)
				return SelectedAttempt{}, evidence, firstErr
			}
			if err != nil && firstErr == nil && !errors.Is(err, context.Canceled) {
				firstErr = err
			}
		}
	}
}

func defaultAttemptPair(
	ctx context.Context,
	demux *udpowner.TraversalDemux,
	plan pairPlan,
	key []byte,
) (AttemptPairResult, error) {
	return attemptPairWithPunch(ctx, demux, plan, key, punching.MakeHole)
}

type makeHoleFunc func(
	ctx context.Context,
	listenConn *net.UDPConn,
	demux *udpowner.TraversalDemux,
	m *legacywire.NatHoleResp,
	key []byte,
) (*net.UDPConn, *net.UDPAddr, error)

func attemptPairWithPunch(
	ctx context.Context,
	demux *udpowner.TraversalDemux,
	plan pairPlan,
	key []byte,
	makeHole makeHoleFunc,
) (AttemptPairResult, error) {
	traceCtx := withAttemptTraceLogger(ctx, plan.sid)
	if directAddr, ok, err := mirroredHostRemoteAddr(plan); err != nil {
		return AttemptPairResult{}, fmt.Errorf("resolve mirrored host path for %s -> %s: %w", plan.local.Addr, plan.remote.Addr, err)
	} else if ok {
		logutil.Infof(
			"punch mirrored host path selected: local_candidate=%s remote_candidate=%s selected_path=%s remote_udp=%s",
			plan.local.Addr,
			plan.remote.Addr,
			PathDirectIPv4,
			directAddr.String(),
		)
		return AttemptPairResult{RemoteAddr: directAddr, Path: PathDirectIPv4, Detail: directAddr.String()}, nil
	}
	if demux == nil {
		return AttemptPairResult{}, errors.New("nil traversal demux")
	}
	if plan.resp == nil {
		return AttemptPairResult{}, errors.New("nil pair response")
	}
	if makeHole == nil {
		return AttemptPairResult{}, errors.New("nil make hole func")
	}

	var directErr error
	if raddr, err := attemptDirectIPv4(traceCtx, demux, plan, key); err == nil {
		return AttemptPairResult{RemoteAddr: raddr, Path: PathDirectIPv4, Detail: raddr.String()}, nil
	} else if !errors.Is(err, errDirectIPv4NotApplicable) {
		directErr = err
	}

	_, raddr, err := makeHole(traceCtx, plan.conn, demux, plan.resp, key)
	if err != nil {
		if directErr != nil {
			return AttemptPairResult{}, fmt.Errorf("direct_ipv4 failed: %v; punching_ipv4 failed: %w", directErr, err)
		}
		return AttemptPairResult{}, fmt.Errorf("make hole for %s -> %s: %w", plan.local.Addr, plan.remote.Addr, err)
	}

	detail := raddr.String()
	if directErr != nil {
		detail = fmt.Sprintf("direct_ipv4=%s: %v; punching_ipv4=%s", attemptResultForError(directErr), directErr, raddr.String())
	}
	return AttemptPairResult{RemoteAddr: raddr, Path: PathPunchingIPv4, Detail: detail}, nil
}

func normalizeAttemptPairResult(result AttemptPairResult) AttemptPairResult {
	result.Path = strings.TrimSpace(result.Path)
	result.Detail = strings.TrimSpace(result.Detail)
	if result.Path == "" {
		result.Path = PathPunchingIPv4
	}
	if result.Detail == "" && result.RemoteAddr != nil {
		result.Detail = result.RemoteAddr.String()
	}
	return result
}

func mirroredHostRemoteAddr(plan pairPlan) (*net.UDPAddr, bool, error) {
	if plan.local.Kind != CandidateKindHost || plan.remote.Kind != CandidateKindHost {
		return nil, false, nil
	}

	localAddr, err := net.ResolveUDPAddr("udp4", plan.local.Addr)
	if err != nil {
		return nil, false, err
	}
	remoteAddr, err := net.ResolveUDPAddr("udp4", plan.remote.Addr)
	if err != nil {
		return nil, false, err
	}

	if localAddr == nil || remoteAddr == nil || localAddr.IP == nil || remoteAddr.IP == nil {
		return nil, false, nil
	}
	if !localAddr.IP.Equal(remoteAddr.IP) {
		return nil, false, nil
	}
	if localAddr.Port == 0 || remoteAddr.Port == 0 || localAddr.Port == remoteAddr.Port {
		return nil, false, nil
	}

	return remoteAddr, true, nil
}

func natHoleRespForPair(remote Candidate, sid string, initiator bool) *legacywire.NatHoleResp {
	role := punching.DetectRoleReceiver
	sendDelayMs := 0
	if initiator {
		role = punching.DetectRoleSender
		sendDelayMs = 150
	}
	return &legacywire.NatHoleResp{
		TransactionID:   sid,
		Sid:             sid,
		PunchingEnabled: true,
		CandidateAddrs:  []string{remote.Addr},
		DetectBehavior: legacywire.NatHoleDetectBehavior{
			Role:          role,
			Mode:          punching.DetectMode0,
			TTL:           0,
			SendDelayMs:   sendDelayMs,
			ReadTimeoutMs: int(defaultPunchReadTimeout.Milliseconds()),
		},
	}
}

func attemptResultForError(err error) string {
	switch {
	case errors.Is(err, context.Canceled):
		return "canceled"
	case isTimeoutError(err):
		return "timeout"
	default:
		return "failed"
	}
}

func sidForDialPair(dialID string, local Candidate, remote Candidate) string {
	left := candidateSIDKey(local)
	right := candidateSIDKey(remote)
	if left > right {
		left, right = right, left
	}
	base := dialID + "|" + left + "|" + right
	sum := sha256.Sum256([]byte(base))
	return base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString(sum[:16])
}

func candidateSIDKey(candidate Candidate) string {
	return string(candidate.Kind) + "|" + candidate.Addr
}
