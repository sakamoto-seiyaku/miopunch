package pocacceptor

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"
	"sync"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/shelllock"
	"github.com/miopunch/miopunch/internal/shellproto"
	"github.com/miopunch/miopunch/internal/shelltarget"
	mqttsig "github.com/miopunch/miopunch/internal/signaling/mqtt"
	"github.com/miopunch/miopunch/internal/wire"
)

type Config struct {
	StatePath string

	LockTTL time.Duration
}

func Run(ctx context.Context, cfg Config) error {
	if strings.TrimSpace(cfg.StatePath) == "" {
		path, err := pocstate.DefaultStatePath()
		if err != nil {
			return err
		}
		cfg.StatePath = path
	}
	if cfg.LockTTL <= 0 {
		cfg.LockTTL = 60 * time.Second
	}

	locks := shelllock.New(cfg.LockTTL)

	ticker := time.NewTicker(2 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return nil
		case <-ticker.C:
		}

		st, err := pocstate.Load(cfg.StatePath)
		if err != nil {
			continue
		}
		if st.Local == nil {
			continue
		}
		st.Local.NormalizeDefaults()
		if strings.TrimSpace(st.Local.ProxyName) == "" || strings.TrimSpace(st.Local.SecretKey) == "" {
			continue
		}

		if err := serveOnce(ctx, st.Local, locks, cfg.LockTTL); err != nil {
			// Best-effort: keep the daemon running; transient errors are expected.
			continue
		}
	}
}

func serveOnce(ctx context.Context, local *pocstate.LocalConfig, locks *shelllock.Manager, lockTTL time.Duration) error {
	handshakeCtx, cancel := context.WithTimeout(ctx, 2*time.Minute)
	defer cancel()

	sid := mqttsig.DeriveSID(local.ProxyName, local.SecretKey)

	gather, err := connectivity.Gather(handshakeCtx, sid, connectivity.GatherConfig{
		ListenPort: 0,
	})
	if err != nil {
		return err
	}
	// Ownership of UDPConn moves to the data plane (stream) on success.
	defer func() {
		if gather.UDP4Conn != nil {
			_ = gather.UDP4Conn.Close()
		}
		if gather.UDP6Conn != nil {
			_ = gather.UDP6Conn.Close()
		}
	}()

	transactionID := time.Now().UTC().UnixNano()

	natHoleClientMsg := &wire.NatHoleClient{
		TransactionID: fmt.Sprintf("tx-%d", transactionID),
		ProxyName:     local.ProxyName,
		Sid:           sid,
		Protocol:      local.DataProto,
		QuicCC:        local.QUICCC,
		DirectAddrs:   gather.DirectAddrs,
		MappedAddrs:   gather.MappedAddrs,
		AssistedAddrs: gather.AssistedAddrs,
		STUNCN:        gather.STUNCN,
		STUNGlobal:    gather.STUNGlobal,
	}

	mq, err := mqttsig.Open(handshakeCtx, mqttsig.Config{
		BrokerURL:       mqttBrokerURL(local.MQTTBroker),
		TopicPrefix:     local.TopicPrefix,
		SID:             sid,
		Role:            mqttsig.RoleClient,
		HelloTimeout:    10 * time.Second,
		ExchangeTimeout: 10 * time.Second,
		BarrierTimeout:  10 * time.Second,
	})
	if err != nil {
		return err
	}

	natHoleRespMsg, err := mq.RunClient(handshakeCtx, natHoleClientMsg)
	_ = mq.Close()
	if err != nil {
		return err
	}

	attemptRes, err := connectivity.Attempt(handshakeCtx, sid, []byte(local.SecretKey), gather.UDP4Conn, gather.UDP6Conn, natHoleRespMsg, connectivity.AttemptConfig{})
	if err != nil {
		return err
	}
	if attemptRes.Conn == gather.UDP4Conn {
		gather.UDP4Conn = nil
	}
	if attemptRes.Conn == gather.UDP6Conn {
		gather.UDP6Conn = nil
	}

	dpCfg := dataplane.Config{
		Proto:  dataplane.Protocol(natHoleRespMsg.Protocol),
		QuicCC: dataplane.QUICCC(natHoleRespMsg.QuicCC),
		Brutal: dataplane.BrutalConfig{
			UpBps:   natHoleRespMsg.BrutalUpBps,
			DownBps: natHoleRespMsg.BrutalDownBps,
		},
	}

	stream, err := dataplane.ServeStream(handshakeCtx, dpCfg, attemptRes.Conn, attemptRes.Remote, nil)
	if err != nil {
		return err
	}
	defer stream.Close()

	reader := shellproto.NewReader(stream)
	writer := shellproto.NewWriter(stream)

	type frameResult struct {
		kind    shellproto.Kind
		payload []byte
		err     error
	}
	frameCh := make(chan frameResult, 1)
	go func() {
		kind, payload, err := reader.ReadFrame()
		frameCh <- frameResult{kind: kind, payload: payload, err: err}
	}()

	var ctl shellproto.Control
	select {
	case res := <-frameCh:
		if res.err != nil {
			return res.err
		}
		if res.kind != shellproto.KindJSON {
			return errors.New("first frame must be JSON")
		}
		if err := json.Unmarshal(res.payload, &ctl); err != nil {
			return err
		}
	case <-handshakeCtx.Done():
		_ = stream.Close()
		return handshakeCtx.Err()
	}

	switch strings.TrimSpace(ctl.Op) {
	case shellproto.OpPing:
		return servePing(writer)
	case shellproto.OpShLS:
		return serveShLS(handshakeCtx, writer, ctl)
	case shellproto.OpShAttach:
		return serveShAttach(ctx, local, locks, lockTTL, reader, writer, stream, ctl)
	default:
		return errors.New("unknown op")
	}
}

func servePing(w *shellproto.Writer) error {
	return w.WriteJSON(shellproto.Control{Op: shellproto.OpPing, OK: true})
}

func serveShLS(ctx context.Context, w *shellproto.Writer, req shellproto.Control) error {
	targets, err := shelltarget.ListTargets(ctx)
	if err != nil {
		return w.WriteJSON(shellproto.Control{
			Op: shellproto.OpShLS,
			OK: false,
			Error: &shellproto.ControlError{
				ReasonCode: "SH_CONNECTOR_FAIL",
				Message:    err.Error(),
			},
		})
	}

	rawTarget := strings.TrimSpace(req.Target)
	if rawTarget == "" {
		return w.WriteJSON(shellproto.Control{Op: shellproto.OpShLS, OK: true, Targets: targets})
	}

	target, err := shelltarget.Resolve(rawTarget, targets)
	if err != nil {
		return w.WriteJSON(shellproto.Control{
			Op: shellproto.OpShLS,
			OK: false,
			Error: &shellproto.ControlError{
				ReasonCode: reasonCodeForTargetErr(err),
				Message:    err.Error(),
			},
		})
	}

	sessions, err := shelltarget.ListSessions(ctx, target)
	if err != nil {
		reason := "SH_CONNECTOR_FAIL"
		var suggestions []string
		if errors.Is(err, shelltarget.ErrTmuxMissing) {
			reason = "SH_TMUX_MISSING"
			suggestions = tmuxMissingSuggestions(target)
		}
		return w.WriteJSON(shellproto.Control{
			Op:     shellproto.OpShLS,
			OK:     false,
			Target: target,
			Error: &shellproto.ControlError{
				ReasonCode:  reason,
				Message:     err.Error(),
				Suggestions: suggestions,
			},
		})
	}

	return w.WriteJSON(shellproto.Control{Op: shellproto.OpShLS, OK: true, Target: target, Sessions: sessions})
}

func serveShAttach(
	ctx context.Context,
	local *pocstate.LocalConfig,
	locks *shelllock.Manager,
	lockTTL time.Duration,
	r *shellproto.Reader,
	w *shellproto.Writer,
	stream io.ReadWriteCloser,
	req shellproto.Control,
) error {
	targets, err := shelltarget.ListTargets(ctx)
	if err != nil {
		_ = w.WriteJSON(shellproto.Control{
			Op: shellproto.OpShAttach,
			OK: false,
			Error: &shellproto.ControlError{
				ReasonCode: "SH_CONNECTOR_FAIL",
				Message:    err.Error(),
			},
		})
		return err
	}

	target, err := shelltarget.Resolve(req.Target, targets)
	if err != nil {
		_ = w.WriteJSON(shellproto.Control{
			Op: shellproto.OpShAttach,
			OK: false,
			Error: &shellproto.ControlError{
				ReasonCode: reasonCodeForTargetErr(err),
				Message:    err.Error(),
			},
		})
		return err
	}

	session := strings.TrimSpace(req.Session)
	if session == "" {
		session = "main"
	}

	lock, err := locks.Acquire(shelllock.Key{
		PeerID:  strings.TrimSpace(local.PeerID),
		Target:  target,
		Session: session,
	})
	if err != nil {
		if errors.Is(err, shelllock.ErrInUse) {
			_ = w.WriteJSON(shellproto.Control{
				Op: shellproto.OpShAttach,
				OK: false,
				Error: &shellproto.ControlError{
					ReasonCode: "SH_IN_USE",
					Message:    "session in use",
					Suggestions: []string{
						"retry later",
						"use a different session name",
					},
				},
			})
		}
		return err
	}
	defer lock.Release()

	ptySess, err := shelltarget.Attach(ctx, target, session)
	if err != nil {
		reason := "SH_CONNECTOR_FAIL"
		var suggestions []string
		if errors.Is(err, shelltarget.ErrTmuxMissing) {
			reason = "SH_TMUX_MISSING"
			suggestions = tmuxMissingSuggestions(target)
		}
		_ = w.WriteJSON(shellproto.Control{
			Op: shellproto.OpShAttach,
			OK: false,
			Error: &shellproto.ControlError{
				ReasonCode:  reason,
				Message:     err.Error(),
				Suggestions: suggestions,
			},
		})
		return err
	}
	defer ptySess.Close()

	if req.WinSize != nil {
		_ = ptySess.Resize(req.WinSize.Cols, req.WinSize.Rows)
	}

	exitCh := make(chan error, 1)
	go func() { exitCh <- ptySess.Wait() }()
	select {
	case err := <-exitCh:
		_ = w.WriteJSON(shellproto.Control{
			Op: shellproto.OpShAttach,
			OK: false,
			Error: &shellproto.ControlError{
				ReasonCode: "SH_TMUX_ATTACH_FAIL",
				Message:    err.Error(),
			},
		})
		return err
	case <-time.After(200 * time.Millisecond):
	}

	if err := w.WriteJSON(shellproto.Control{Op: shellproto.OpShAttach, OK: true, Target: target, Session: session}); err != nil {
		return err
	}
	lock.Touch()

	bridgeCtx, cancel := context.WithCancel(ctx)
	defer cancel()

	var closeOnce sync.Once
	closeAll := func(cause error) {
		_ = cause
		closeOnce.Do(func() {
			cancel()
			_ = stream.Close()
			_ = ptySess.Close()
		})
	}

	type outFrame struct {
		kind    shellproto.Kind
		payload []byte
	}
	outCh := make(chan outFrame, 32)

	activityCh := make(chan struct{}, 1)
	signalActivity := func() {
		lock.Touch()
		select {
		case activityCh <- struct{}{}:
		default:
		}
	}

	go func() {
		timer := time.NewTimer(lockTTL)
		defer timer.Stop()
		for {
			select {
			case <-bridgeCtx.Done():
				return
			case <-timer.C:
				closeAll(errors.New("idle timeout"))
				return
			case <-activityCh:
				if !timer.Stop() {
					select {
					case <-timer.C:
					default:
					}
				}
				timer.Reset(lockTTL)
			}
		}
	}()

	go func() {
		defer closeAll(nil)
		buf := make([]byte, 32*1024)
		for {
			n, err := ptySess.Read(buf)
			if n > 0 {
				payload := append([]byte(nil), buf[:n]...)
				select {
				case outCh <- outFrame{kind: shellproto.KindData, payload: payload}:
				case <-bridgeCtx.Done():
					return
				}
			}
			if err != nil {
				return
			}
		}
	}()

	go func() {
		ticker := time.NewTicker(shellproto.DefaultHeartbeatInterval)
		defer ticker.Stop()
		for {
			select {
			case <-bridgeCtx.Done():
				return
			case <-ticker.C:
				data, _ := json.Marshal(shellproto.Control{Op: shellproto.OpHeartbeat})
				select {
				case outCh <- outFrame{kind: shellproto.KindJSON, payload: data}:
				case <-bridgeCtx.Done():
					return
				}
			}
		}
	}()

	go func() {
		for {
			select {
			case <-bridgeCtx.Done():
				return
			case frame := <-outCh:
				if err := shellproto.WriteFrame(stream, frame.kind, frame.payload); err != nil {
					closeAll(err)
					return
				}
				signalActivity()
			}
		}
	}()

	for {
		kind, payload, err := r.ReadFrame()
		if err != nil {
			closeAll(err)
			return err
		}
		switch kind {
		case shellproto.KindData:
			if _, err := ptySess.Write(payload); err != nil {
				closeAll(err)
				return err
			}
			signalActivity()
		case shellproto.KindJSON:
			var ctl shellproto.Control
			if err := json.Unmarshal(payload, &ctl); err != nil {
				continue
			}
			switch strings.TrimSpace(ctl.Op) {
			case shellproto.OpWinSize:
				if ctl.WinSize != nil {
					_ = ptySess.Resize(ctl.WinSize.Cols, ctl.WinSize.Rows)
				}
				signalActivity()
			case shellproto.OpHeartbeat:
				signalActivity()
			default:
				// Ignore unknown control operations for forward compatibility.
				signalActivity()
			}
		default:
			// Ignore unknown frame kinds.
			signalActivity()
		}
	}
}

func reasonCodeForTargetErr(err error) string {
	switch err.(type) {
	case shelltarget.TargetNotFoundError:
		return "SH_TARGET_NOT_FOUND"
	case shelltarget.TargetAmbiguousError:
		return "SH_TARGET_AMBIGUOUS"
	default:
		return "SH_CONNECTOR_FAIL"
	}
}

func tmuxMissingSuggestions(target string) []string {
	target = strings.TrimSpace(target)

	switch {
	case target == "local":
		return []string{
			"install tmux on the controlled node (example: sudo apt-get install tmux)",
			"retry after tmux is installed",
		}
	case strings.HasPrefix(target, "wsl:"):
		distro := strings.TrimSpace(strings.TrimPrefix(target, "wsl:"))
		if distro == "" {
			return []string{
				"install tmux inside WSL distro (example: wsl.exe -l -q)",
				"retry after tmux is installed",
			}
		}
		return []string{
			fmt.Sprintf("install tmux inside WSL distro (example: wsl.exe -d %q -- sudo apt-get install tmux)", distro),
			"retry after tmux is installed",
		}
	case strings.HasPrefix(target, "ssh:"):
		host := strings.TrimSpace(strings.TrimPrefix(target, "ssh:"))
		if host == "" {
			return []string{
				"install tmux on SSH host (example: ssh <host> \"sudo apt-get install tmux\")",
				"retry after tmux is installed",
			}
		}
		return []string{
			fmt.Sprintf("install tmux on SSH host (example: ssh %q \"sudo apt-get install tmux\")", host),
			"retry after tmux is installed",
		}
	default:
		return []string{
			"install tmux on the controlled node",
			"retry after tmux is installed",
		}
	}
}

func mqttBrokerURL(broker string) string {
	broker = strings.TrimSpace(broker)
	if broker == "" {
		return ""
	}
	if strings.Contains(broker, "://") {
		return broker
	}
	return "tcp://" + broker
}
