package task

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"
	"time"

	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/shellproto"
)

const (
	sessionKeepaliveInterval = 30 * time.Second
	sessionKeepaliveMinIdle  = 45 * time.Second
	sessionKeepaliveTimeout  = 15 * time.Second
)

// StartSessionKeepalive starts the daemon-level application keepalive loop.
func (m *Manager) StartSessionKeepalive(ctx context.Context) {
	if m == nil || m.sessions == nil {
		return
	}
	m.wg.Add(1)
	go func() {
		defer m.wg.Done()

		ticker := time.NewTicker(sessionKeepaliveInterval)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-m.ctx.Done():
				return
			case <-ticker.C:
				m.keepaliveActiveSessions()
			}
		}
	}()
}

func (m *Manager) keepaliveActiveSessions() {
	if m == nil || m.sessions == nil {
		return
	}

	now := time.Now().UTC()
	for _, summary := range m.sessions.ListSummaries() {
		key := summary.Key.Normalize()
		if key.RemotePeerID == "" || !summary.Healthy {
			continue
		}
		lastActivity := time.UnixMilli(summary.LastActivityUnixMilli).UTC()
		if !lastActivity.IsZero() && now.Sub(lastActivity) < sessionKeepaliveMinIdle {
			continue
		}

		sess, ok := m.sessions.Get(key)
		if !ok {
			continue
		}
		if _, passive := sess.(*passivePeerSession); passive {
			continue
		}
		if err := m.keepaliveSession(sess); err != nil {
			logutil.Warnf("session keepalive failed: peer_id=%s proto=%s path_family=%s err=%v", key.RemotePeerID, key.Protocol, key.PathFamily, err)
			m.sessions.Close(key, dataplane.CloseReasonTransportFatal)
			m.recordTopologyAttempt(TopologyAttempt{
				PeerID:        key.RemotePeerID,
				AttemptPath:   "keepalive",
				AttemptWay:    "keepalive",
				DataProto:     string(key.Protocol),
				PathFamily:    string(key.PathFamily),
				StartedAt:     now.UnixMilli(),
				Outcome:       "fail",
				Stage:         string(poc.StageCapabilityHandshake),
				ReasonCode:    string(poc.ReasonCodeUnavailable),
				StopCondition: "keepalive_failed",
			})
			continue
		}
		m.recordTopologyPayload(TopologyPayload{
			PeerID:     key.RemotePeerID,
			Evidence:   "keepalive=ok",
			ObservedAt: time.Now().UTC().UnixMilli(),
		})
	}
}

func (m *Manager) keepaliveSession(sess dataplane.PeerSession) error {
	if sess == nil {
		return errors.New("nil session")
	}

	ctx, cancel := context.WithTimeout(m.ctx, sessionKeepaliveTimeout)
	defer cancel()

	open, err := m.buildShellStreamOpen(shellproto.OpPing, "", "", "")
	if err != nil {
		return err
	}
	stream, err := sess.OpenStream(ctx, open)
	if err != nil {
		return fmt.Errorf("open keepalive stream: %w", err)
	}
	defer stream.Close()

	if err := m.readKeepaliveHello(ctx, stream); err != nil {
		return err
	}
	if err := shellproto.WriteJSON(stream, shellproto.Control{Op: shellproto.OpPing}); err != nil {
		return fmt.Errorf("send keepalive ping: %w", err)
	}

	kind, payload, err := readFrameWithContext(ctx, stream)
	if err != nil {
		return fmt.Errorf("read keepalive response: %w", err)
	}
	if kind != shellproto.KindJSON {
		return fmt.Errorf("unexpected keepalive response kind: %d", kind)
	}

	var resp shellproto.Control
	if err := json.Unmarshal(payload, &resp); err != nil {
		return fmt.Errorf("invalid keepalive response json: %w", err)
	}
	if strings.TrimSpace(resp.Op) != shellproto.OpPing || !resp.OK {
		return errors.New("keepalive ping rejected")
	}
	return nil
}

func (m *Manager) readKeepaliveHello(ctx context.Context, stream io.ReadWriteCloser) error {
	kind, payload, err := readFrameWithContext(ctx, stream)
	if err != nil {
		return fmt.Errorf("read keepalive hello: %w", err)
	}
	if kind != shellproto.KindJSON {
		return fmt.Errorf("unexpected keepalive hello kind: %d", kind)
	}

	var resp shellproto.Control
	if err := json.Unmarshal(payload, &resp); err != nil {
		return fmt.Errorf("invalid keepalive hello json: %w", err)
	}
	if strings.TrimSpace(resp.Op) != shellproto.OpHello || !resp.OK {
		return errors.New("keepalive hello rejected")
	}
	if err := m.mergeHelloResponseDecls(resp); err != nil {
		return fmt.Errorf("merge keepalive hello decls: %w", err)
	}
	return nil
}
