package pocacceptor

import (
	"context"
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"
	"sync"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/controlplane"
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

		if err := serveOnce(ctx, cfg.StatePath, st.Local, locks, cfg.LockTTL); err != nil {
			// Best-effort: keep the daemon running; transient errors are expected.
			continue
		}
	}
}

func serveOnce(ctx context.Context, statePath string, local *pocstate.LocalConfig, locks *shelllock.Manager, lockTTL time.Duration) error {
	handshakeCtx, cancel := context.WithTimeout(ctx, 2*time.Minute)
	defer cancel()

	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		return err
	}

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

	reader := shellproto.NewReader(stream)
	writer := shellproto.NewWriter(stream)

	closeStream := true
	defer func() {
		if closeStream {
			_ = stream.Close()
		}
	}()

	readControl := func(ctx context.Context) (shellproto.Control, error) {
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
				return shellproto.Control{}, res.err
			}
			if res.kind != shellproto.KindJSON {
				return shellproto.Control{}, errors.New("frame must be JSON")
			}
			if err := json.Unmarshal(res.payload, &ctl); err != nil {
				return shellproto.Control{}, err
			}
			return ctl, nil
		case <-ctx.Done():
			_ = stream.Close()
			return shellproto.Control{}, ctx.Err()
		}
	}

	helloCtl, err := readControl(handshakeCtx)
	if err != nil {
		return err
	}
	if strings.TrimSpace(helloCtl.Op) != shellproto.OpHello {
		_ = writer.WriteJSON(shellproto.Control{
			Op: shellproto.OpHello,
			OK: false,
			Error: &shellproto.ControlError{
				ReasonCode: shellproto.ReasonHelloRequired,
				Message:    "hello required",
				Suggestions: []string{
					"upgrade both ends to POC-06.5 hello handshake",
					"ensure you have joined and are approved before ping/sh",
				},
			},
		})
		return errors.New("hello required")
	}
	if err := handleHello(handshakeCtx, stateDir, writer, helloCtl); err != nil {
		return err
	}

	ctl, err := readControl(handshakeCtx)
	if err != nil {
		return err
	}

	switch strings.TrimSpace(ctl.Op) {
	case shellproto.OpPing:
		return servePing(writer)
	case shellproto.OpShLS:
		return serveShLS(handshakeCtx, writer, ctl)
	case shellproto.OpShAttach:
		closeStream = false
		go func() {
			_ = serveShAttach(ctx, local, locks, lockTTL, reader, writer, stream, ctl)
			_ = stream.Close()
		}()
		return nil
	default:
		return errors.New("unknown op")
	}
}

func handleHello(ctx context.Context, stateDir string, w *shellproto.Writer, req shellproto.Control) error {
	_ = ctx
	if w == nil {
		return errors.New("nil hello writer")
	}

	peerID, err := controlplane.CanonicalizePeerID(req.PeerID)
	if err != nil {
		_ = writeHelloError(w, shellproto.ReasonHelloInvalid, "invalid peer_id", []string{"join and retry"})
		return err
	}
	sigB64 := strings.TrimSpace(req.SigB64)
	if sigB64 == "" {
		_ = writeHelloError(w, shellproto.ReasonHelloInvalid, "missing sig_b64", []string{"join and retry"})
		return errors.New("missing hello sig_b64")
	}

	head, err := pocstate.LoadGovernanceHeadSnapshot(stateDir)
	if err != nil {
		_ = writeHelloError(w, shellproto.ReasonHelloInternal, "missing governance head snapshot", []string{"initialize governance via: miopunch invite"})
		return err
	}

	declsFile, err := pocstate.EnsureDecls(stateDir)
	if err != nil {
		_ = writeHelloError(w, shellproto.ReasonHelloInternal, "failed to load decls", []string{"retry"})
		return err
	}

	revoked, err := isRevokedV0(head, declsFile.Decls, peerID)
	if err != nil {
		_ = writeHelloError(w, shellproto.ReasonHelloInternal, "failed to validate local decls", []string{"retry"})
		return err
	}
	if revoked && !head.IsAdmin(peerID) && !head.IsOwner(peerID) {
		_ = writeHelloError(w, shellproto.ReasonHelloRevoked, "peer_id is revoked", []string{"re-join with a new identity"})
		return errors.New("revoked peer")
	}

	approveMsgID := ""
	var memberEdPub ed25519.PublicKey
	var candidateDecl pocstate.DeclV0
	haveCandidateDecl := false

	if len(req.ApproveDecl) > 0 {
		var decl pocstate.DeclV0
		if err := json.Unmarshal(req.ApproveDecl, &decl); err != nil {
			_ = writeHelloError(w, shellproto.ReasonHelloDeclInvalid, "invalid approve_decl", []string{"join and retry"})
			return err
		}
		if strings.TrimSpace(decl.Kind) != pocstate.DeclKindApproveMember {
			_ = writeHelloError(w, shellproto.ReasonHelloDeclInvalid, "approve_decl kind mismatch", []string{"join and retry"})
			return errors.New("approve_decl kind mismatch")
		}

		issuerPub, ok, err := head.AdminEd25519Pub(decl.IssuerPeerID)
		if err != nil {
			_ = writeHelloError(w, shellproto.ReasonHelloDeclInvalid, "invalid approve_decl issuer_peer_id", []string{"join and retry"})
			return err
		}
		if !ok {
			_ = writeHelloError(w, shellproto.ReasonHelloIssuerNotAdmin, "approve_decl issuer is not an admin", []string{"join and retry"})
			return errors.New("approve_decl issuer not admin")
		}
		if err := pocstate.VerifyDeclV0(issuerPub, decl); err != nil {
			_ = writeHelloError(w, shellproto.ReasonHelloDeclInvalid, "approve_decl signature invalid", []string{"re-run join and retry"})
			return err
		}

		var body pocstate.ApproveMemberBodyV0
		if err := json.Unmarshal(decl.Body, &body); err != nil {
			_ = writeHelloError(w, shellproto.ReasonHelloDeclInvalid, "approve_decl body invalid", []string{"join and retry"})
			return err
		}
		if strings.TrimSpace(body.MemberPeerID) != peerID {
			_ = writeHelloError(w, shellproto.ReasonHelloDeclInvalid, "approve_decl member_peer_id mismatch", []string{"join and retry"})
			return errors.New("approve_decl member_peer_id mismatch")
		}

		memberEdPubBytes, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(body.Ed25519PubB64))
		if err != nil || len(memberEdPubBytes) != ed25519.PublicKeySize {
			_ = writeHelloError(w, shellproto.ReasonHelloDeclInvalid, "approve_decl member ed25519_pub_b64 invalid", []string{"join and retry"})
			return errors.New("approve_decl member ed25519_pub_b64 invalid")
		}
		memberEdPub = ed25519.PublicKey(memberEdPubBytes)

		derivedPeerID, err := controlplane.PeerIDFromEd25519Pub(memberEdPub)
		if err != nil {
			_ = writeHelloError(w, shellproto.ReasonHelloDeclInvalid, "approve_decl member pubkey invalid", []string{"join and retry"})
			return err
		}
		if derivedPeerID != peerID {
			_ = writeHelloError(w, shellproto.ReasonHelloDeclInvalid, "approve_decl member pubkey does not match peer_id", []string{"join and retry"})
			return errors.New("approve_decl member pubkey mismatch")
		}

		approveMsgID = decl.MsgID
		candidateDecl = decl
		haveCandidateDecl = true
	} else if head.IsAdmin(peerID) {
		pub, ok, err := head.AdminEd25519Pub(peerID)
		if err != nil {
			_ = writeHelloError(w, shellproto.ReasonHelloInternal, "failed to load admin pubkey", []string{"retry"})
			return err
		}
		if !ok {
			_ = writeHelloError(w, shellproto.ReasonHelloNotApproved, "peer_id not approved", []string{"join and retry"})
			return errors.New("admin pubkey missing")
		}
		memberEdPub = pub
	} else {
		pub, ok, err := findApprovedMemberPubV0(head, declsFile.Decls, peerID)
		if err != nil {
			_ = writeHelloError(w, shellproto.ReasonHelloInternal, "failed to validate local decls", []string{"retry"})
			return err
		}
		if !ok {
			_ = writeHelloError(w, shellproto.ReasonHelloNotApproved, "peer_id not approved", []string{"join and retry"})
			return errors.New("peer not approved")
		}
		memberEdPub = pub
	}

	if err := shellproto.VerifyHelloV0(memberEdPub, peerID, approveMsgID, sigB64); err != nil {
		_ = writeHelloError(w, shellproto.ReasonHelloSigInvalid, "invalid hello signature", []string{"ensure you are using the correct identity"})
		return err
	}

	if haveCandidateDecl {
		if _, err := pocstate.UpdateDecls(stateDir, func(f *pocstate.DeclsFileV0) error {
			f.Decls = pocstate.AddDeclSetUnionV0(f.Decls, candidateDecl)
			return nil
		}); err != nil {
			_ = writeHelloError(w, shellproto.ReasonHelloInternal, "failed to persist decls", []string{"retry"})
			return err
		}
	}

	return w.WriteJSON(shellproto.Control{Op: shellproto.OpHello, OK: true})
}

func writeHelloError(w *shellproto.Writer, reasonCode string, message string, suggestions []string) error {
	if w == nil {
		return errors.New("nil hello writer")
	}
	message = strings.TrimSpace(message)
	if message == "" {
		message = "hello rejected"
	}
	out := shellproto.Control{
		Op: shellproto.OpHello,
		OK: false,
		Error: &shellproto.ControlError{
			ReasonCode:  strings.TrimSpace(reasonCode),
			Message:     message,
			Suggestions: suggestions,
		},
	}
	return w.WriteJSON(out)
}

func isRevokedV0(head pocstate.GovernanceHeadSnapshotV1, decls []pocstate.DeclV0, peerID string) (bool, error) {
	peerID = strings.TrimSpace(peerID)
	if peerID == "" {
		return false, errors.New("empty peer_id")
	}

	for _, d := range decls {
		if strings.TrimSpace(d.Kind) != pocstate.DeclKindRevokeMember {
			continue
		}

		issuerPub, ok, err := head.AdminEd25519Pub(d.IssuerPeerID)
		if err != nil {
			return false, err
		}
		if !ok {
			continue
		}
		if err := pocstate.VerifyDeclV0(issuerPub, d); err != nil {
			continue
		}

		var body pocstate.RevokeMemberBodyV0
		if err := json.Unmarshal(d.Body, &body); err != nil {
			continue
		}
		if strings.TrimSpace(body.MemberPeerID) == peerID {
			return true, nil
		}
	}
	return false, nil
}

func findApprovedMemberPubV0(head pocstate.GovernanceHeadSnapshotV1, decls []pocstate.DeclV0, peerID string) (ed25519.PublicKey, bool, error) {
	peerID = strings.TrimSpace(peerID)
	if peerID == "" {
		return nil, false, errors.New("empty peer_id")
	}

	for _, d := range decls {
		if strings.TrimSpace(d.Kind) != pocstate.DeclKindApproveMember {
			continue
		}

		issuerPub, ok, err := head.AdminEd25519Pub(d.IssuerPeerID)
		if err != nil {
			return nil, false, err
		}
		if !ok {
			continue
		}
		if err := pocstate.VerifyDeclV0(issuerPub, d); err != nil {
			continue
		}

		var body pocstate.ApproveMemberBodyV0
		if err := json.Unmarshal(d.Body, &body); err != nil {
			continue
		}
		if strings.TrimSpace(body.MemberPeerID) != peerID {
			continue
		}

		memberEdPubBytes, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(body.Ed25519PubB64))
		if err != nil || len(memberEdPubBytes) != ed25519.PublicKeySize {
			continue
		}
		return ed25519.PublicKey(memberEdPubBytes), true, nil
	}

	return nil, false, nil
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
