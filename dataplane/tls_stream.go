package dataplane

import (
	"context"
	"crypto/rand"
	"crypto/tls"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/hashicorp/yamux"
	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/tlsutil"
)

const (
	tlsRoleVisitor = "visitor"
	tlsRoleClient  = "client"
)

const (
	tlsElectionKeepPrefix = "miopunch/tls-election/v0:keep:"
	tlsElectionMaxFrame   = 512
	tlsElectionTimeout    = 2 * time.Second
	tlsHandshakeTimeout   = 6 * time.Second
	tlsHandshakeSettle    = 200 * time.Millisecond
)

type tlsCandidate struct {
	Conn   *tls.Conn
	Origin connectivity.TCPConnOrigin
	Index  int
}

func tlsConnSummary(conn net.Conn) (local string, remote string) {
	if conn == nil {
		return "", ""
	}
	return conn.LocalAddr().String(), conn.RemoteAddr().String()
}

func tlsContextDeadlineSummary(ctx context.Context) (hasDeadline bool, deadline string, remainingMS int64, err error) {
	err = ctx.Err()
	d, ok := ctx.Deadline()
	if !ok {
		return false, "", 0, err
	}
	return true, d.Format(time.RFC3339Nano), time.Until(d).Milliseconds(), err
}

func DialTLSStream(ctx context.Context, sid string, secretKey []byte, candidates []connectivity.TCPConn, em *event.Emitter) (io.ReadWriteCloser, error) {
	cfg := Config{
		Proto:      ProtocolTLS,
		SecurityID: sid,
		SecretKey:  secretKey,
		PathFamily: PathFamilyTCP4,
	}
	sess, err := DialTLSSession(ctx, cfg, candidates, em)
	if err != nil {
		return nil, err
	}
	stream, err := sess.OpenStream(ctx, StreamOpen{Kind: StreamKindShellV0})
	if err != nil {
		_ = sess.Close(CloseReasonTransportFatal)
		return nil, err
	}
	return &sessionOwnedStream{ReadWriteCloser: stream, session: sess}, nil
}

func ServeTLSStream(ctx context.Context, sid string, secretKey []byte, candidates []connectivity.TCPConn, em *event.Emitter) (io.ReadWriteCloser, error) {
	cfg := Config{
		Proto:      ProtocolTLS,
		SecurityID: sid,
		SecretKey:  secretKey,
		PathFamily: PathFamilyTCP4,
	}
	sess, err := ServeTLSSession(ctx, cfg, candidates, em)
	if err != nil {
		return nil, err
	}
	accepted, err := sess.AcceptStream(ctx)
	if err != nil {
		_ = sess.Close(CloseReasonTransportFatal)
		return nil, err
	}
	if accepted.Open.Kind != StreamKindShellV0 {
		_ = accepted.Stream.Close()
		_ = sess.Close(CloseReasonStreamProtocolError)
		return nil, fmt.Errorf("unexpected stream kind: %q", accepted.Open.Kind)
	}
	return &sessionOwnedStream{ReadWriteCloser: accepted.Stream, session: sess}, nil
}

func DialAndExchangeTLS(ctx context.Context, cfg Config, sid string, secretKey []byte, candidates []connectivity.TCPConn, payload []byte, em *event.Emitter) error {
	if err := cfg.Validate(); err != nil {
		return err
	}
	if cfg.Proto != ProtocolTLS {
		return fmt.Errorf("tls exchange requires data proto %q, got %q", ProtocolTLS, cfg.Proto)
	}

	cfg.SecurityID = sid
	cfg.SecretKey = secretKey
	cfg.PathFamily = PathFamilyTCP4
	stream, err := dialPayloadTLSStream(ctx, cfg, candidates, em)
	if err != nil {
		return fmt.Errorf("dial payload tls stream: %w", err)
	}
	defer stream.Close()

	conn, ok := stream.(net.Conn)
	if ok {
		_ = conn.SetDeadline(time.Now().Add(15 * time.Second))
		defer conn.SetDeadline(time.Time{})
	}

	if err := writeFrame(stream, payload); err != nil {
		// With yamux, it's possible for the peer to close the session after it has
		// already received enough bytes to finish its read path. In that case the
		// request may have been delivered even though the final write observes
		// session shutdown due to a concurrent close.
		if !errors.Is(err, yamux.ErrSessionShutdown) {
			return fmt.Errorf("write request frame: %w", err)
		}
	}
	resp, err := readFrame(stream, 64*1024)
	if err != nil {
		return fmt.Errorf("read response frame: %w", err)
	}
	if string(resp) != "ok:"+string(payload) {
		return fmt.Errorf("unexpected response: %q", string(resp))
	}

	emitPayloadExchanged(em, cfg, len(payload), "tls")
	return nil
}

func ServeAndExchangeTLS(ctx context.Context, cfg Config, sid string, secretKey []byte, candidates []connectivity.TCPConn, em *event.Emitter) error {
	if err := cfg.Validate(); err != nil {
		return err
	}
	if cfg.Proto != ProtocolTLS {
		return fmt.Errorf("tls exchange requires data proto %q, got %q", ProtocolTLS, cfg.Proto)
	}

	cfg.SecurityID = sid
	cfg.SecretKey = secretKey
	cfg.PathFamily = PathFamilyTCP4
	stream, err := servePayloadTLSStream(ctx, cfg, candidates, em)
	if err != nil {
		return fmt.Errorf("serve payload tls stream: %w", err)
	}
	defer stream.Close()

	conn, ok := stream.(net.Conn)
	if ok {
		_ = conn.SetDeadline(time.Now().Add(15 * time.Second))
		defer conn.SetDeadline(time.Time{})
	}

	req, err := readFrame(stream, 64*1024)
	if err != nil {
		return fmt.Errorf("read request frame: %w", err)
	}
	resp := append([]byte("ok:"), req...)
	if err := writeFrame(stream, resp); err != nil {
		// With yamux, it's possible for the peer to close the session immediately
		// after it has already received enough bytes to finish its read path.
		// In that case the response is effectively delivered, but the final write
		// may observe session shutdown due to the session closing concurrently.
		if errors.Is(err, yamux.ErrSessionShutdown) {
			emitPayloadExchanged(em, cfg, len(req), "tls")
			return nil
		}
		return fmt.Errorf("write response frame: %w", err)
	}

	emitPayloadExchanged(em, cfg, len(req), "tls")
	return nil
}

func dialPayloadTLSStream(ctx context.Context, cfg Config, candidates []connectivity.TCPConn, em *event.Emitter) (io.ReadWriteCloser, error) {
	sess, err := DialTLSSession(ctx, cfg, candidates, em)
	if err != nil {
		return nil, err
	}
	stream, err := sess.OpenStream(ctx, StreamOpen{Kind: StreamKindPayloadV0})
	if err != nil {
		_ = sess.Close(CloseReasonTransportFatal)
		return nil, err
	}
	return &sessionOwnedStream{ReadWriteCloser: stream, session: sess}, nil
}

func servePayloadTLSStream(ctx context.Context, cfg Config, candidates []connectivity.TCPConn, em *event.Emitter) (io.ReadWriteCloser, error) {
	sess, err := ServeTLSSession(ctx, cfg, candidates, em)
	if err != nil {
		return nil, err
	}
	accepted, err := sess.AcceptStream(ctx)
	if err != nil {
		_ = sess.Close(CloseReasonTransportFatal)
		return nil, err
	}
	if accepted.Open.Kind != StreamKindPayloadV0 {
		_ = accepted.Stream.Close()
		_ = sess.Close(CloseReasonStreamProtocolError)
		return nil, fmt.Errorf("unexpected stream kind: %q", accepted.Open.Kind)
	}
	return &sessionOwnedStream{ReadWriteCloser: accepted.Stream, session: sess}, nil
}

func convergePinnedTLS(ctx context.Context, sid string, secretKey []byte, selfRole string, peerRole string, asClient bool, candidates []connectivity.TCPConn, em *event.Emitter) (*tls.Conn, error) {
	candidates = slices.DeleteFunc(slices.Clone(candidates), func(c connectivity.TCPConn) bool {
		return c.Conn == nil
	})
	if len(candidates) == 0 {
		return nil, errors.New("no tcp connections for tls")
	}
	logutil.Infof(
		"tcp tls converge start: sid=%s self_role=%s peer_role=%s as_client=%v candidates=%d",
		sid,
		selfRole,
		peerRole,
		asClient,
		len(candidates),
	)

	var tlsConfig *tls.Config
	var err error
	if asClient {
		tlsConfig, err = tlsutil.NewPinnedClientTLSConfig(secretKey, sid, selfRole, peerRole)
	} else {
		tlsConfig, err = tlsutil.NewPinnedServerTLSConfig(secretKey, sid, selfRole, peerRole)
	}
	if err != nil {
		closeTCPCandidates(candidates)
		return nil, err
	}

	handshakeCtx, cancel := context.WithTimeout(ctx, tlsHandshakeTimeout)
	defer cancel()
	parentHasDeadline, parentDeadline, parentRemainingMS, parentErr := tlsContextDeadlineSummary(ctx)
	handshakeHasDeadline, handshakeDeadline, handshakeRemainingMS, handshakeErr := tlsContextDeadlineSummary(handshakeCtx)
	logutil.Infof(
		"tcp tls converge deadlines: sid=%s role=%s parent_has_deadline=%v parent_deadline=%s parent_remaining_ms=%d parent_err=%v handshake_has_deadline=%v handshake_deadline=%s handshake_remaining_ms=%d handshake_err=%v",
		sid,
		selfRole,
		parentHasDeadline,
		parentDeadline,
		parentRemainingMS,
		parentErr,
		handshakeHasDeadline,
		handshakeDeadline,
		handshakeRemainingMS,
		handshakeErr,
	)

	resultCh := make(chan tlsCandidate, len(candidates))
	doneCh := make(chan struct{})
	convergeStarted := time.Now()

	var wg sync.WaitGroup
	for i, cand := range candidates {
		i, cand := i, cand
		wg.Add(1)
		go func() {
			defer wg.Done()

			base := tlsConfig.Clone()
			var tlsConn *tls.Conn
			local, remote := tlsConnSummary(cand.Conn)
			started := time.Now()
			logutil.Infof(
				"tcp tls handshake start: sid=%s role=%s origin=%s local=%s remote=%s as_client=%v",
				sid,
				selfRole,
				cand.Origin,
				local,
				remote,
				asClient,
			)
			if asClient {
				tlsConn = tls.Client(cand.Conn, base)
			} else {
				tlsConn = tls.Server(cand.Conn, base)
			}

			if err := tlsConn.HandshakeContext(handshakeCtx); err != nil {
				logutil.Warnf(
					"tcp tls handshake failed: sid=%s role=%s origin=%s local=%s remote=%s elapsed_ms=%d err=%v",
					sid,
					selfRole,
					cand.Origin,
					local,
					remote,
					time.Since(started).Milliseconds(),
					err,
				)
				_ = tlsConn.Close()
				return
			}
			local, remote = tlsConnSummary(tlsConn)
			logutil.Infof(
				"tcp tls handshake ok: sid=%s role=%s origin=%s local=%s remote=%s elapsed_ms=%d",
				sid,
				selfRole,
				cand.Origin,
				local,
				remote,
				time.Since(started).Milliseconds(),
			)

			select {
			case resultCh <- tlsCandidate{Conn: tlsConn, Origin: cand.Origin, Index: i}:
			default:
				logutil.Warnf(
					"tcp tls handshake result dropped: sid=%s role=%s origin=%s local=%s remote=%s",
					sid,
					selfRole,
					cand.Origin,
					local,
					remote,
				)
				_ = tlsConn.Close()
			}
		}()
	}

	go func() {
		wg.Wait()
		close(doneCh)
	}()

	successes, firstSuccessElapsedMS := collectTLSHandshakeSuccesses(
		handshakeCtx,
		cancel,
		doneCh,
		resultCh,
		len(candidates),
		convergeStarted,
	)
	if len(successes) == 0 {
		err := handshakeCtx.Err()
		if err == nil {
			err = errors.New("tls handshake failed for all candidates")
		}
		logutil.Warnf(
			"tcp tls converge failed: sid=%s role=%s candidates=%d successes=0 err=%v",
			sid,
			selfRole,
			len(candidates),
			err,
		)
		return nil, err
	}
	closePendingTCPCandidates(candidates, successes)
	logutil.Infof(
		"tcp tls converge handshakes ready: sid=%s role=%s candidates=%d successes=%d first_success_elapsed_ms=%d settle_window_ms=%d",
		sid,
		selfRole,
		len(candidates),
		len(successes),
		firstSuccessElapsedMS,
		tlsHandshakeSettle.Milliseconds(),
	)

	electionCtx, cancelElection := context.WithTimeout(ctx, tlsElectionTimeout)
	defer cancelElection()
	electionHasDeadline, electionDeadline, electionRemainingMS, electionErr := tlsContextDeadlineSummary(electionCtx)
	logutil.Infof(
		"tcp tls election deadlines: sid=%s role=%s election_has_deadline=%v election_deadline=%s election_remaining_ms=%d election_err=%v",
		sid,
		selfRole,
		electionHasDeadline,
		electionDeadline,
		electionRemainingMS,
		electionErr,
	)

	winner, winnerOrigin, err := convergePinnedTLSElection(electionCtx, sid, successes, selfRole)
	if err != nil {
		logutil.Warnf(
			"tcp tls election failed: sid=%s role=%s successes=%d err=%v",
			sid,
			selfRole,
			len(successes),
			err,
		)
		for _, c := range successes {
			_ = c.Conn.Close()
		}
		return nil, err
	}

	if em != nil {
		em.Emit(event.Event{
			Stage: event.StageTransport,
			Kind:  event.KindInfo,
			Name:  "transport.tls_converge",
			Msg:   "tls winner converged",
			KVs: map[string]any{
				"candidates":               len(candidates),
				"successes":                len(successes),
				"strategy":                 "leader_follower",
				"role":                     selfRole,
				"winner":                   string(winnerOrigin),
				"first_success_elapsed_ms": firstSuccessElapsedMS,
				"settle_window_ms":         int(tlsHandshakeSettle.Milliseconds()),
			},
		})
	}

	return winner, nil
}

func closeTCPCandidates(candidates []connectivity.TCPConn) {
	for _, c := range candidates {
		if c.Conn != nil {
			_ = c.Conn.Close()
		}
	}
}

func closePendingTCPCandidates(candidates []connectivity.TCPConn, successes []tlsCandidate) {
	if len(candidates) == 0 {
		return
	}
	keep := make(map[int]struct{}, len(successes))
	for _, success := range successes {
		keep[success.Index] = struct{}{}
	}
	for i, cand := range candidates {
		if _, ok := keep[i]; ok {
			continue
		}
		if cand.Conn != nil {
			_ = cand.Conn.Close()
		}
	}
}

func collectTLSHandshakeSuccesses(
	ctx context.Context,
	cancel context.CancelFunc,
	doneCh <-chan struct{},
	resultCh <-chan tlsCandidate,
	total int,
	started time.Time,
) ([]tlsCandidate, int64) {
	successes := make([]tlsCandidate, 0, total)
	var firstSuccessElapsedMS int64
	var settleTimer *time.Timer
	var settleC <-chan time.Time

	drain := func() bool {
		drained := false
		for {
			select {
			case res := <-resultCh:
				if res.Conn == nil {
					continue
				}
				successes = append(successes, res)
				if len(successes) == 1 {
					firstSuccessElapsedMS = time.Since(started).Milliseconds()
					settleTimer = time.NewTimer(tlsHandshakeSettle)
					settleC = settleTimer.C
				}
				drained = true
			default:
				return drained
			}
		}
	}

	for {
		if len(successes) > 0 && settleC == nil {
			firstSuccessElapsedMS = time.Since(started).Milliseconds()
			settleTimer = time.NewTimer(tlsHandshakeSettle)
			settleC = settleTimer.C
		}
		select {
		case res := <-resultCh:
			if res.Conn == nil {
				continue
			}
			successes = append(successes, res)
			if len(successes) == 1 {
				firstSuccessElapsedMS = time.Since(started).Milliseconds()
				settleTimer = time.NewTimer(tlsHandshakeSettle)
				settleC = settleTimer.C
			}
		case <-settleC:
			if settleTimer != nil {
				settleTimer.Stop()
			}
			cancel()
			drain()
			return successes, firstSuccessElapsedMS
		case <-doneCh:
			if settleTimer != nil {
				settleTimer.Stop()
			}
			drain()
			return successes, firstSuccessElapsedMS
		case <-ctx.Done():
			if settleTimer != nil {
				settleTimer.Stop()
			}
			drain()
			return successes, firstSuccessElapsedMS
		}
	}
}

func convergePinnedTLSElection(ctx context.Context, sid string, successes []tlsCandidate, selfRole string) (*tls.Conn, connectivity.TCPConnOrigin, error) {
	switch selfRole {
	case tlsRoleVisitor:
		return convergePinnedTLSLeader(ctx, sid, successes)
	case tlsRoleClient:
		return convergePinnedTLSFollower(ctx, sid, successes)
	default:
		return nil, "", fmt.Errorf("unknown tls role: %q", selfRole)
	}
}

func convergePinnedTLSLeader(ctx context.Context, sid string, successes []tlsCandidate) (*tls.Conn, connectivity.TCPConnOrigin, error) {
	token := make([]byte, 16)
	if _, err := rand.Read(token); err != nil {
		return nil, "", fmt.Errorf("tls election token: %w", err)
	}
	tokenHex := hex.EncodeToString(token)
	keepMsg := []byte(tlsElectionKeepPrefix + tokenHex)

	var deadline time.Time
	if d, ok := ctx.Deadline(); ok {
		deadline = d
	} else {
		deadline = time.Now().Add(tlsElectionTimeout)
	}
	logutil.Infof(
		"tcp tls election leader deadline selected: sid=%s deadline=%s remaining_ms=%d ctx_err=%v",
		sid,
		deadline.Format(time.RFC3339Nano),
		time.Until(deadline).Milliseconds(),
		ctx.Err(),
	)

	var winnerConn *tls.Conn
	var winnerOrigin connectivity.TCPConnOrigin
	for _, c := range successes {
		_ = c.Conn.SetWriteDeadline(deadline)
		local, remote := tlsConnSummary(c.Conn)
		logutil.Infof(
			"tcp tls election leader signal start: sid=%s origin=%s local=%s remote=%s frame_len=%d deadline=%s remaining_ms=%d",
			sid,
			c.Origin,
			local,
			remote,
			len(keepMsg),
			deadline.Format(time.RFC3339Nano),
			time.Until(deadline).Milliseconds(),
		)
		err := writeFrame(c.Conn, keepMsg)
		_ = c.Conn.SetWriteDeadline(time.Time{})
		if err != nil {
			logutil.Warnf(
				"tcp tls election leader signal failed: sid=%s origin=%s local=%s remote=%s err=%v",
				sid,
				c.Origin,
				local,
				remote,
				err,
			)
			_ = c.Conn.Close()
			continue
		}
		logutil.Infof(
			"tcp tls election leader signal ok: sid=%s origin=%s local=%s remote=%s",
			sid,
			c.Origin,
			local,
			remote,
		)
		winnerConn = c.Conn
		winnerOrigin = c.Origin
		break
	}
	if winnerConn == nil {
		return nil, "", errors.New("tls election failed: leader could not signal any winner")
	}

	for _, c := range successes {
		if c.Conn == winnerConn {
			continue
		}
		_ = c.Conn.Close()
	}
	_ = winnerConn.SetDeadline(time.Time{})
	return winnerConn, winnerOrigin, nil
}

func convergePinnedTLSFollower(ctx context.Context, sid string, successes []tlsCandidate) (*tls.Conn, connectivity.TCPConnOrigin, error) {
	var deadline time.Time
	if d, ok := ctx.Deadline(); ok {
		deadline = d
	} else {
		deadline = time.Now().Add(tlsElectionTimeout)
	}
	logutil.Infof(
		"tcp tls election follower deadline selected: sid=%s deadline=%s remaining_ms=%d ctx_err=%v",
		sid,
		deadline.Format(time.RFC3339Nano),
		time.Until(deadline).Milliseconds(),
		ctx.Err(),
	)

	type outcome struct {
		cand  tlsCandidate
		frame []byte
		err   error
	}

	outCh := make(chan outcome, len(successes))
	var wg sync.WaitGroup
	for _, cand := range successes {
		cand := cand
		wg.Add(1)
		go func() {
			defer wg.Done()
			local, remote := tlsConnSummary(cand.Conn)
			logutil.Infof(
				"tcp tls election follower read start: sid=%s origin=%s local=%s remote=%s deadline=%s remaining_ms=%d",
				sid,
				cand.Origin,
				local,
				remote,
				deadline.Format(time.RFC3339Nano),
				time.Until(deadline).Milliseconds(),
			)
			_ = cand.Conn.SetReadDeadline(deadline)
			frame, err := readFrame(cand.Conn, tlsElectionMaxFrame)
			if err != nil {
				logutil.Warnf(
					"tcp tls election follower read failed: sid=%s origin=%s local=%s remote=%s err=%v",
					sid,
					cand.Origin,
					local,
					remote,
					err,
				)
			} else {
				logutil.Infof(
					"tcp tls election follower read ok: sid=%s origin=%s local=%s remote=%s frame_len=%d keep_prefix=%v",
					sid,
					cand.Origin,
					local,
					remote,
					len(frame),
					strings.HasPrefix(string(frame), tlsElectionKeepPrefix),
				)
			}
			outCh <- outcome{cand: cand, frame: frame, err: err}
		}()
	}

	go func() {
		wg.Wait()
		close(outCh)
	}()

	var winnerConn *tls.Conn
	var winnerOrigin connectivity.TCPConnOrigin

	for res := range outCh {
		if res.err != nil {
			_ = res.cand.Conn.Close()
			continue
		}
		msg := string(res.frame)
		if strings.HasPrefix(msg, tlsElectionKeepPrefix) && winnerConn == nil {
			winnerConn = res.cand.Conn
			winnerOrigin = res.cand.Origin
			for _, c := range successes {
				if c.Conn != winnerConn {
					_ = c.Conn.Close()
				}
			}
			continue
		}
		_ = res.cand.Conn.Close()
	}

	if winnerConn == nil {
		return nil, "", errors.New("tls election failed: follower did not receive winner signal")
	}

	_ = winnerConn.SetDeadline(time.Time{})
	return winnerConn, winnerOrigin, nil
}
