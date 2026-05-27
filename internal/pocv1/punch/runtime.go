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
	"sync"

	"github.com/miopunch/miopunch/internal/logutil"
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
	remoteCandidates []Candidate,
	initiator bool,
) (PathResult, error) {
	plans, err := buildPairPlans(cfg.UDPConn, cfg.LocalCandidates, remoteCandidates, dialID, initiator)
	if err != nil {
		return PathResult{}, wrapDiagnosticError(Diagnostic{
			DialID:             dialID,
			RemotePeerID:       remote.PeerID,
			LocalCandidates:    cfg.LocalCandidates,
			RemoteCandidates:   remoteCandidates,
			AttemptConcurrency: cfg.AttemptConcurrency,
			AttemptBudget:      cfg.AttemptBudget,
		}, err)
	}
	logutil.Debugf(
		"punch run start: dial_id=%s remote_peer_id=%s local_candidates=%d remote_candidates=%d planned_pairs=%d attempt_concurrency=%d attempt_budget_ms=%d initiator=%t",
		dialID,
		remote.PeerID,
		len(cfg.LocalCandidates),
		len(remoteCandidates),
		len(plans),
		cfg.AttemptConcurrency,
		cfg.AttemptBudget.Milliseconds(),
		initiator,
	)
	attemptCtx, cancel := withAttemptBudget(ctx, cfg.AttemptBudget)
	defer cancel()

	demux, err := udpowner.NewUDPTraversalDemux(cfg.UDPConn, udpowner.DemuxConfig{Key: punchToken})
	if err != nil {
		return PathResult{}, wrapDiagnosticError(Diagnostic{
			DialID:             dialID,
			RemotePeerID:       remote.PeerID,
			LocalCandidates:    cfg.LocalCandidates,
			RemoteCandidates:   remoteCandidates,
			PlannedPairCount:   len(plans),
			AttemptConcurrency: cfg.AttemptConcurrency,
			AttemptBudget:      cfg.AttemptBudget,
		}, fmt.Errorf("open udp traversal demux: %w", err))
	}
	defer demux.Close()

	selected, evidence, err := executePairPlans(attemptCtx, cfg, demux, plans, punchToken)
	if err != nil {
		logutil.Warnf(
			"punch run failed: dial_id=%s remote_peer_id=%s err=%v attempted_pairs=%s",
			dialID,
			remote.PeerID,
			err,
			summarizeAttemptResults(evidence),
		)
		return PathResult{}, wrapDiagnosticError(Diagnostic{
			DialID:             dialID,
			RemotePeerID:       remote.PeerID,
			LocalCandidates:    cfg.LocalCandidates,
			RemoteCandidates:   remoteCandidates,
			PlannedPairCount:   len(plans),
			AttemptConcurrency: cfg.AttemptConcurrency,
			AttemptBudget:      cfg.AttemptBudget,
			AttemptedPairs:     evidence,
		}, err)
	}
	logutil.Infof(
		"punch run selected: dial_id=%s remote_peer_id=%s local_candidate=%s remote_candidate=%s remote_udp=%s attempted_pairs=%s",
		dialID,
		remote.PeerID,
		selected.LocalCandidate.Addr,
		selected.RemoteCandidate.Addr,
		selected.RemoteAddr.String(),
		summarizeAttemptResults(evidence),
	)

	return PathResult{
		Conn:       selected.Conn,
		RemoteAddr: selected.RemoteAddr,
		RemoteIdentity: TrustedRemoteIdentity{
			PeerID:           remote.PeerID,
			MemberCredential: append([]byte(nil), remote.MemberCredential...),
		},
		Evidence: PunchEvidence{
			DialID:            dialID,
			AttemptedPairs:    evidence,
			SelectedLocal:     selected.LocalCandidate,
			SelectedRemote:    selected.RemoteCandidate,
			SelectedRemoteUDP: selected.RemoteAddr.String(),
		},
	}, nil
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

			raddr, err := cfg.AttemptPair(attemptCtx, demux, plan, key)
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

			evidence[plan.index].Result = "selected"
			evidence[plan.index].Detail = raddr.String()
			logutil.Infof(
				"punch attempt selected: sid=%s local_candidate=%s remote_candidate=%s remote_udp=%s",
				plan.sid,
				plan.local.Addr,
				plan.remote.Addr,
				raddr.String(),
			)
			select {
			case resultCh <- SelectedAttempt{
				LocalCandidate:  plan.local,
				RemoteCandidate: plan.remote,
				Conn:            plan.conn,
				RemoteAddr:      raddr,
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
) (*net.UDPAddr, error) {
	if demux == nil {
		return nil, errors.New("nil traversal demux")
	}
	if plan.resp == nil {
		return nil, errors.New("nil pair response")
	}

	_, raddr, err := punching.MakeHole(ctx, plan.conn, demux, plan.resp, key)
	if err != nil {
		return nil, fmt.Errorf("make hole for %s -> %s: %w", plan.local.Addr, plan.remote.Addr, err)
	}
	return raddr, nil
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
