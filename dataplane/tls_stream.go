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
)

type tlsCandidate struct {
	Conn   *tls.Conn
	Origin connectivity.TCPConnOrigin
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

	handshakeCtx, cancel := context.WithTimeout(ctx, 6*time.Second)
	defer cancel()

	resultCh := make(chan tlsCandidate, len(candidates))

	var wg sync.WaitGroup
	for _, cand := range candidates {
		cand := cand
		wg.Add(1)
		go func() {
			defer wg.Done()

			base := tlsConfig.Clone()
			var tlsConn *tls.Conn
			if asClient {
				tlsConn = tls.Client(cand.Conn, base)
			} else {
				tlsConn = tls.Server(cand.Conn, base)
			}

			if err := tlsConn.HandshakeContext(handshakeCtx); err != nil {
				_ = tlsConn.Close()
				return
			}

			select {
			case resultCh <- tlsCandidate{Conn: tlsConn, Origin: cand.Origin}:
			default:
				_ = tlsConn.Close()
			}
		}()
	}

	go func() {
		wg.Wait()
		close(resultCh)
	}()

	successes := make([]tlsCandidate, 0, len(candidates))
	for res := range resultCh {
		if res.Conn == nil {
			continue
		}
		successes = append(successes, res)
	}
	if len(successes) == 0 {
		err := handshakeCtx.Err()
		if err == nil {
			err = errors.New("tls handshake failed for all candidates")
		}
		return nil, err
	}

	electionCtx, cancelElection := context.WithTimeout(ctx, tlsElectionTimeout)
	defer cancelElection()

	winner, winnerOrigin, err := convergePinnedTLSElection(electionCtx, successes, selfRole)
	if err != nil {
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
				"candidates": len(candidates),
				"successes":  len(successes),
				"strategy":   "leader_follower",
				"role":       selfRole,
				"winner":     string(winnerOrigin),
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

func convergePinnedTLSElection(ctx context.Context, successes []tlsCandidate, selfRole string) (*tls.Conn, connectivity.TCPConnOrigin, error) {
	switch selfRole {
	case tlsRoleVisitor:
		return convergePinnedTLSLeader(ctx, successes)
	case tlsRoleClient:
		return convergePinnedTLSFollower(ctx, successes)
	default:
		return nil, "", fmt.Errorf("unknown tls role: %q", selfRole)
	}
}

func convergePinnedTLSLeader(ctx context.Context, successes []tlsCandidate) (*tls.Conn, connectivity.TCPConnOrigin, error) {
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

	var winnerConn *tls.Conn
	var winnerOrigin connectivity.TCPConnOrigin
	for _, c := range successes {
		_ = c.Conn.SetWriteDeadline(deadline)
		err := writeFrame(c.Conn, keepMsg)
		_ = c.Conn.SetWriteDeadline(time.Time{})
		if err != nil {
			_ = c.Conn.Close()
			continue
		}
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

func convergePinnedTLSFollower(ctx context.Context, successes []tlsCandidate) (*tls.Conn, connectivity.TCPConnOrigin, error) {
	var deadline time.Time
	if d, ok := ctx.Deadline(); ok {
		deadline = d
	} else {
		deadline = time.Now().Add(tlsElectionTimeout)
	}

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
			_ = cand.Conn.SetReadDeadline(deadline)
			frame, err := readFrame(cand.Conn, tlsElectionMaxFrame)
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
