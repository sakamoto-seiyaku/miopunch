package pocacceptor

import (
	"context"
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/apernet/quic-go"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/shelllock"
	"github.com/miopunch/miopunch/internal/shellproto"
	"github.com/miopunch/miopunch/internal/shelltarget"
	mqttsig "github.com/miopunch/miopunch/internal/signaling/mqtt"
	"github.com/miopunch/miopunch/internal/udpowner"
	"github.com/miopunch/miopunch/internal/wire"
)

const daemonPortMapSessionLease = 30 * time.Minute

type Config struct {
	StatePath string

	LockTTL time.Duration
}

type peerSessionRegistry struct {
	mu     sync.Mutex
	byPeer map[string]map[dataplane.PeerSession]struct{}
}

func newPeerSessionRegistry() *peerSessionRegistry {
	return &peerSessionRegistry{
		byPeer: make(map[string]map[dataplane.PeerSession]struct{}),
	}
}

func (r *peerSessionRegistry) Add(peerID string, sess dataplane.PeerSession) {
	if r == nil || strings.TrimSpace(peerID) == "" || sess == nil {
		return
	}
	peerID = strings.TrimSpace(peerID)
	r.mu.Lock()
	m := r.byPeer[peerID]
	if m == nil {
		m = make(map[dataplane.PeerSession]struct{})
		r.byPeer[peerID] = m
	}
	m[sess] = struct{}{}
	r.mu.Unlock()
}

func (r *peerSessionRegistry) Remove(peerID string, sess dataplane.PeerSession) {
	if r == nil || strings.TrimSpace(peerID) == "" || sess == nil {
		return
	}
	peerID = strings.TrimSpace(peerID)
	r.mu.Lock()
	m := r.byPeer[peerID]
	delete(m, sess)
	if len(m) == 0 {
		delete(r.byPeer, peerID)
	}
	r.mu.Unlock()
}

func (r *peerSessionRegistry) ClosePeer(peerID string, reason dataplane.CloseReason) {
	if r == nil || strings.TrimSpace(peerID) == "" {
		return
	}
	peerID = strings.TrimSpace(peerID)

	r.mu.Lock()
	m := r.byPeer[peerID]
	delete(r.byPeer, peerID)
	sessions := make([]dataplane.PeerSession, 0, len(m))
	for sess := range m {
		sessions = append(sessions, sess)
	}
	r.mu.Unlock()

	for _, sess := range sessions {
		_ = sess.Close(reason)
	}
}

func (r *peerSessionRegistry) Replace(peerID string, sess dataplane.PeerSession, closeReason dataplane.CloseReason) {
	if r == nil || strings.TrimSpace(peerID) == "" || sess == nil {
		return
	}
	peerID = strings.TrimSpace(peerID)

	r.mu.Lock()
	old := r.byPeer[peerID]
	next := make(map[dataplane.PeerSession]struct{}, 1)
	next[sess] = struct{}{}
	r.byPeer[peerID] = next

	toClose := make([]dataplane.PeerSession, 0, len(old))
	for oldSess := range old {
		if oldSess == nil || oldSess == sess {
			continue
		}
		toClose = append(toClose, oldSess)
	}
	r.mu.Unlock()

	for _, oldSess := range toClose {
		_ = oldSess.Close(closeReason)
	}
}

// taskGroup is a small helper to avoid sync.WaitGroup "Add called concurrently with Wait"
// panics during shutdown. It blocks new spawns once Wait is entered.
//
// goroutines MUST NOT call Go() after shutdown begins.
type taskGroup struct {
	mu      sync.Mutex
	wg      sync.WaitGroup
	waiting bool
}

func (g *taskGroup) Go(fn func()) {
	if fn == nil {
		return
	}
	g.mu.Lock()
	if g.waiting {
		g.mu.Unlock()
		return
	}
	g.wg.Add(1)
	g.mu.Unlock()

	go func() {
		defer g.wg.Done()
		fn()
	}()
}

func (g *taskGroup) Wait() {
	g.mu.Lock()
	g.waiting = true
	g.mu.Unlock()
	g.wg.Wait()
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
	stateDir, err := pocstate.StateDir(statePath)
	if err != nil {
		return err
	}

	sid := mqttsig.DeriveSID(local.ProxyName, local.SecretKey)
	localPeerID := strings.TrimSpace(local.PeerID)
	logutil.Infof(
		"pocacceptor starting: peer_id=%s sid=%s %s",
		localPeerID,
		sid,
		safeAcceptorMQTTSummary(local),
	)

	serveCtx, cancel := context.WithCancel(ctx)
	defer cancel()

	// Gather owns long-lived resources such as portmap cleanup hooks; tie it to
	// the acceptor lifetime instead of a short discovery timeout.
	gatherCtx, cancelGather := context.WithCancel(serveCtx)
	defer cancelGather()

	gather, err := connectivity.Gather(gatherCtx, sid, connectivity.GatherConfig{
		ListenPort:           local.P2PPort,
		P2PNetwork:           connectivity.P2PNetwork(local.P2PNetwork),
		P2PIPFamily:          connectivity.P2PIPFamily(local.P2PIPFamily),
		DisableAssistedAddrs: local.DisableAssistedAddrs,
		DisablePortMap:       local.DisablePortMap,
		StunServers:          local.StunServers,
		StunExplicit:         local.StunExplicit,
		SessionLease:         daemonPortMapSessionLease,
	})
	if err != nil {
		logutil.Warnf("pocacceptor gather failed: peer_id=%s sid=%s err=%v", localPeerID, sid, err)
		return err
	}
	logutil.Infof(
		"pocacceptor gather ready: peer_id=%s sid=%s udp4=%t udp6=%t tcp4=%t tcp6=%t direct=%d mapped=%d assisted=%d tcp_direct=%d tcp_mapped=%d tcp_assisted=%d",
		localPeerID,
		sid,
		gather.UDP4Conn != nil,
		gather.UDP6Conn != nil,
		gather.TCP4Listener != nil,
		gather.TCP6Listener != nil,
		len(gather.DirectAddrs),
		len(gather.MappedAddrs),
		len(gather.AssistedAddrs),
		len(gather.TCPDirectAddrs),
		len(gather.TCPMappedAddrs),
		len(gather.TCPAssistedAddrs),
	)
	// Ownership of UDPConn moves to the data plane (stream) on success.
	defer func() {
		if gather.UDP4Conn != nil {
			_ = gather.UDP4Conn.Close()
		}
		if gather.UDP6Conn != nil {
			_ = gather.UDP6Conn.Close()
		}
		if gather.TCP4Listener != nil {
			_ = gather.TCP4Listener.Close()
		}
		if gather.TCP6Listener != nil {
			_ = gather.TCP6Listener.Close()
		}
	}()

	var brutalUpBps, brutalDownBps uint64
	if local.DataProto == "quic" && local.QUICCC == "brutal" {
		brutalUpBps = 1_000_000
		brutalDownBps = 1_000_000
	}

	natHoleClientTemplate := &wire.NatHoleClient{
		ProxyName:        local.ProxyName,
		Sid:              sid,
		Protocol:         local.DataProto,
		QuicCC:           local.QUICCC,
		BrutalUpBps:      brutalUpBps,
		BrutalDownBps:    brutalDownBps,
		Capabilities:     []string{wire.CapabilityTCPP2PV0},
		P2PNetwork:       local.P2PNetwork,
		DirectAddrs:      gather.DirectAddrs,
		MappedAddrs:      gather.MappedAddrs,
		AssistedAddrs:    gather.AssistedAddrs,
		TCPDirectAddrs:   gather.TCPDirectAddrs,
		TCPAssistedAddrs: gather.TCPAssistedAddrs,
		TCPMappedAddrs:   gather.TCPMappedAddrs,
		TCPSTUNCN:        gather.TCPSTUNCN,
		TCPSTUNGlobal:    gather.TCPSTUNGlobal,
		STUNCN:           gather.STUNCN,
		STUNGlobal:       gather.STUNGlobal,
	}

	runtimeBrokers := local.MQTTBrokerEndpoints()
	if len(runtimeBrokers) == 0 {
		return errors.New("missing mqtt_broker in local state")
	}

	var (
		mq           *mqttsig.Session
		activeBroker string
		openFailures []string
	)
	for _, broker := range runtimeBrokers {
		mq, err = mqttsig.Open(serveCtx, mqttsig.Config{
			BrokerURL:       mqttBrokerURL(broker),
			TopicPrefix:     local.TopicPrefix,
			SID:             sid,
			Role:            mqttsig.RoleClient,
			HelloTimeout:    10 * time.Second,
			ExchangeTimeout: 10 * time.Second,
			BarrierTimeout:  10 * time.Second,
		})
		if err == nil {
			activeBroker = broker
			break
		}
		openFailures = append(openFailures, fmt.Sprintf("%s: %v", broker, err))
	}
	if mq == nil {
		logutil.Warnf("pocacceptor mqtt open failed: peer_id=%s sid=%s brokers=%s err=%s", localPeerID, sid, strings.Join(runtimeBrokers, ","), strings.Join(openFailures, "; "))
		return brokerFailuresErrorForLog(openFailures, "mqtt open failed")
	}
	defer mq.Close()
	logutil.Infof("pocacceptor mqtt ready: peer_id=%s sid=%s broker=%s %s", localPeerID, sid, activeBroker, safeAcceptorMQTTSummary(local))

	reg := newPeerSessionRegistry()

	var tg taskGroup
	defer func() {
		cancel()
		tg.Wait()
	}()

	tg.Go(func() {
		watchRevocations(serveCtx, stateDir, reg)
	})

	// UDP data plane listeners are long-lived (multi-peer). Traversal is
	// multiplexed on the same socket via a demux boundary.
	var (
		udp4Demux *udpowner.TraversalDemux
		udp6Demux *udpowner.TraversalDemux

		udp4Listener dataplane.PeerSessionListener
		udp6Listener dataplane.PeerSessionListener

		kcp4Owner *udpowner.KCPOwner
		kcp6Owner *udpowner.KCPOwner
	)

	defer func() {
		if udp4Listener != nil {
			_ = udp4Listener.Close()
		}
		if udp6Listener != nil {
			_ = udp6Listener.Close()
		}
		if udp4Demux != nil {
			_ = udp4Demux.Close()
		}
		if udp6Demux != nil {
			_ = udp6Demux.Close()
		}
		if kcp4Owner != nil {
			_ = kcp4Owner.Close()
		}
		if kcp6Owner != nil {
			_ = kcp6Owner.Close()
		}
	}()

	startAcceptLoop := func(ln dataplane.PeerSessionListener) {
		if ln == nil {
			return
		}
		tg.Go(func() {
			for {
				sess, err := ln.Accept(serveCtx)
				if err != nil {
					if serveCtx.Err() == nil {
						logutil.Warnf("pocacceptor session accept failed: peer_id=%s sid=%s err=%v", localPeerID, sid, err)
					}
					return
				}
				s := sess
				tg.Go(func() {
					servePeerSession(serveCtx, stateDir, local, locks, lockTTL, s, reg)
				})
			}
		})
	}

	secretKey := []byte(local.SecretKey)

	setupUDP := func(conn *net.UDPConn, family dataplane.PathFamily) (*udpowner.TraversalDemux, dataplane.PeerSessionListener, error) {
		if conn == nil {
			return nil, nil, nil
		}
		dpCfg := dataplane.Config{
			Proto:      dataplane.Protocol(local.DataProto),
			QuicCC:     dataplane.QUICCC(local.QUICCC),
			SecurityID: sid,
			SecretKey:  secretKey,
			PathFamily: family,
			Brutal: dataplane.BrutalConfig{
				UpBps:   brutalUpBps,
				DownBps: brutalDownBps,
			},
		}

		switch strings.TrimSpace(local.DataProto) {
		case "quic":
			tr := &quic.Transport{Conn: conn}
			demux, err := udpowner.NewQUICTraversalDemux(tr, udpowner.DemuxConfig{Key: secretKey})
			if err != nil {
				return nil, nil, err
			}
			ln, err := dataplane.ListenSessionsWithQUICTransport(serveCtx, dpCfg, tr, conn, nil)
			if err != nil {
				_ = demux.Close()
				return nil, nil, err
			}
			return demux, ln, nil
		case "kcp":
			owner, err := udpowner.NewKCPOwner(conn, udpowner.KCPOwnerConfig{
				Traversal: udpowner.DemuxConfig{Key: secretKey},
			})
			if err != nil {
				return nil, nil, err
			}
			ln, err := dataplane.ListenSessionsWithKCPPacketConn(serveCtx, dpCfg, owner.PacketConn(), nil)
			if err != nil {
				_ = owner.Close()
				return nil, nil, err
			}
			// Keep owner reachable for Close() ordering (owner closes the UDPConn).
			if family == dataplane.PathFamilyUDP6 {
				kcp6Owner = owner
			} else {
				kcp4Owner = owner
			}
			return owner.TraversalDemux(), ln, nil
		default:
			return nil, nil, fmt.Errorf("unsupported udp data proto: %q", local.DataProto)
		}
	}

	if gather.UDP4Conn != nil {
		d, ln, err := setupUDP(gather.UDP4Conn, dataplane.PathFamilyUDP4)
		if err != nil {
			logutil.Warnf("pocacceptor udp listener setup failed: peer_id=%s sid=%s path_family=udp4 err=%v", localPeerID, sid, err)
			return err
		}
		udp4Demux = d
		udp4Listener = ln
		startAcceptLoop(udp4Listener)
		logutil.Infof("pocacceptor udp listener ready: peer_id=%s sid=%s path_family=udp4", localPeerID, sid)
	}
	if gather.UDP6Conn != nil {
		d, ln, err := setupUDP(gather.UDP6Conn, dataplane.PathFamilyUDP6)
		if err != nil {
			logutil.Warnf("pocacceptor udp listener setup failed: peer_id=%s sid=%s path_family=udp6 err=%v", localPeerID, sid, err)
			return err
		}
		udp6Demux = d
		udp6Listener = ln
		startAcceptLoop(udp6Listener)
		logutil.Infof("pocacceptor udp listener ready: peer_id=%s sid=%s path_family=udp6", localPeerID, sid)
	}

	attemptCfg := connectivity.AttemptConfig{
		P2PNetwork:         connectivity.P2PNetwork(local.P2PNetwork),
		P2PIPFamily:        connectivity.P2PIPFamily(local.P2PIPFamily),
		UDP4TraversalDemux: udp4Demux,
		UDP6TraversalDemux: udp6Demux,
	}

	handleAttempt := func(at mqttsig.ClientAttempt) {
		// Handler must be fast: do the heavy lifting in a goroutine.
		tg.Go(func() {
			logutil.Infof(
				"pocacceptor incoming attempt: peer_id=%s sid=%s dial_id=%s",
				localPeerID,
				sid,
				strings.TrimSpace(at.DialID),
			)
			if at.Err != nil {
				logutil.Warnf("pocacceptor incoming attempt failed: peer_id=%s sid=%s dial_id=%s err=%v", localPeerID, sid, strings.TrimSpace(at.DialID), at.Err)
				return
			}
			if at.ClientResp == nil {
				logutil.Warnf("pocacceptor incoming attempt failed: peer_id=%s sid=%s dial_id=%s err=nil client response", localPeerID, sid, strings.TrimSpace(at.DialID))
				return
			}

			attemptCtx, cancel := context.WithTimeout(serveCtx, 2*time.Minute)
			defer cancel()

			attemptRes, err := connectivity.Attempt(attemptCtx, sid, []byte(local.SecretKey), gather.UDP4Conn, gather.UDP6Conn, gather.TCP4Listener, gather.TCP6Listener, at.ClientResp, attemptCfg)
			if err != nil {
				logutil.Warnf(
					"pocacceptor connectivity attempt failed: peer_id=%s sid=%s dial_id=%s protocol=%s selected_view=%s selected_reason=%s err=%v",
					localPeerID,
					sid,
					strings.TrimSpace(at.DialID),
					strings.TrimSpace(at.ClientResp.Protocol),
					strings.TrimSpace(at.ClientResp.SelectedView),
					strings.TrimSpace(at.ClientResp.SelectedReason),
					err,
				)
				return
			}
			logutil.Infof(
				"pocacceptor connectivity attempt ready: peer_id=%s sid=%s dial_id=%s path=%s tcp_conns=%d protocol=%s selected_view=%s selected_reason=%s",
				localPeerID,
				sid,
				strings.TrimSpace(at.DialID),
				strings.TrimSpace(attemptRes.Path),
				len(attemptRes.TCPConns),
				strings.TrimSpace(at.ClientResp.Protocol),
				strings.TrimSpace(at.ClientResp.SelectedView),
				strings.TrimSpace(at.ClientResp.SelectedReason),
			)

			dpCfg := dataplane.Config{
				Proto:      dataplane.Protocol(at.ClientResp.Protocol),
				QuicCC:     dataplane.QUICCC(at.ClientResp.QuicCC),
				SecurityID: sid,
				SecretKey:  []byte(local.SecretKey),
				PathFamily: dataplane.PathFamilyFromAttemptPath(attemptRes.Path),
				Brutal: dataplane.BrutalConfig{
					UpBps:   at.ClientResp.BrutalUpBps,
					DownBps: at.ClientResp.BrutalDownBps,
				},
			}

			var sess dataplane.PeerSession
			if len(attemptRes.TCPConns) > 0 {
				dpCfg.Proto = dataplane.ProtocolTLS
				sess, err = dataplane.ServeTLSSession(attemptCtx, dpCfg, attemptRes.TCPConns, nil)
			} else {
				// UDP sessions are accepted by the long-lived UDP listener started above.
				logutil.Infof("pocacceptor connectivity attempt delegated: peer_id=%s sid=%s dial_id=%s path=%s", localPeerID, sid, strings.TrimSpace(at.DialID), strings.TrimSpace(attemptRes.Path))
				return
			}
			if err != nil {
				logutil.Warnf("pocacceptor tls session failed: peer_id=%s sid=%s dial_id=%s path=%s err=%v", localPeerID, sid, strings.TrimSpace(at.DialID), strings.TrimSpace(attemptRes.Path), err)
				return
			}

			servePeerSession(serveCtx, stateDir, local, locks, lockTTL, sess, reg)
		})
	}

	if err := mq.ServeClient(serveCtx, natHoleClientTemplate, handleAttempt); err != nil {
		cancel()
		if ctx.Err() == nil {
			logutil.Warnf("pocacceptor mqtt serve failed: peer_id=%s sid=%s broker=%s err=%v", localPeerID, sid, activeBroker, err)
		}
		return err
	}

	return nil
}

func watchRevocations(ctx context.Context, stateDir string, reg *peerSessionRegistry) {
	ticker := time.NewTicker(500 * time.Millisecond)
	defer ticker.Stop()

	prevRevoked := make(map[string]struct{})
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		}

		head, err := pocstate.LoadGovernanceHeadSnapshot(stateDir)
		if err != nil {
			continue
		}
		declsFile, err := pocstate.EnsureDecls(stateDir)
		if err != nil {
			continue
		}

		revokedNow := make(map[string]struct{})
		for _, d := range declsFile.Decls {
			if strings.TrimSpace(d.Kind) != pocstate.DeclKindRevokeMember {
				continue
			}

			issuerPub, ok, err := head.AdminEd25519Pub(d.IssuerPeerID)
			if err != nil || !ok {
				continue
			}
			if err := pocstate.VerifyDeclV0(issuerPub, d); err != nil {
				continue
			}

			var body pocstate.RevokeMemberBodyV0
			if err := json.Unmarshal(d.Body, &body); err != nil {
				continue
			}
			memberID := strings.TrimSpace(body.MemberPeerID)
			if memberID == "" {
				continue
			}
			revokedNow[memberID] = struct{}{}
		}

		for peerID := range revokedNow {
			if _, ok := prevRevoked[peerID]; ok {
				continue
			}
			// Newly observed revocation: cut off sessions immediately.
			reg.ClosePeer(peerID, dataplane.CloseReasonAuthorizationRevocation)
		}
		prevRevoked = revokedNow
	}
}

func servePeerSession(
	ctx context.Context,
	stateDir string,
	local *pocstate.LocalConfig,
	locks *shelllock.Manager,
	lockTTL time.Duration,
	sess dataplane.PeerSession,
	reg *peerSessionRegistry,
) {
	if sess == nil {
		return
	}
	key := sess.Key()

	var streamWG sync.WaitGroup
	defer streamWG.Wait()
	defer sess.Close(dataplane.CloseReasonDaemonShutdown)

	var (
		peerOnce sync.Once
		peerID   string
	)

	bindPeer := func(id string) {
		id = strings.TrimSpace(id)
		if id == "" {
			return
		}
		peerOnce.Do(func() {
			peerID = id
			reg.Replace(id, sess, dataplane.CloseReasonSessionSuperseded)
		})
	}
	defer func() {
		if strings.TrimSpace(peerID) != "" {
			reg.Remove(peerID, sess)
		}
	}()

	for {
		accepted, err := sess.AcceptStream(ctx)
		if err != nil {
			// AcceptStream errors are expected on shutdown, but should be surfaced
			// in lab artifacts during debugging.
			if ctx.Err() == nil {
				logutil.Warnf("pocacceptor accept stream failed: proto=%s sid=%s path_family=%s err=%v", key.Protocol, key.SecurityID, key.PathFamily, err)
			}
			return
		}
		streamWG.Add(1)
		go func(accepted *dataplane.AcceptedStream) {
			defer streamWG.Done()
			if err := serveAcceptedShellStream(ctx, stateDir, local, locks, lockTTL, sess, accepted, bindPeer, reg); err != nil && ctx.Err() == nil {
				kind := ""
				meta := map[string]string(nil)
				if accepted != nil {
					kind = string(accepted.Open.Kind)
					meta = accepted.Open.Metadata
				}
				logutil.Warnf("pocacceptor accepted stream failed: proto=%s sid=%s path_family=%s kind=%s metadata=%s err=%v", key.Protocol, key.SecurityID, key.PathFamily, kind, safeStreamMetadataSummary(meta), err)
			}
		}(accepted)
	}
}

func safeStreamMetadataSummary(meta map[string]string) string {
	if len(meta) == 0 {
		return "keys=[]"
	}
	keys := make([]string, 0, len(meta))
	for k := range meta {
		keys = append(keys, strings.TrimSpace(k))
	}
	sort.Strings(keys)
	return fmt.Sprintf(
		"keys=%v op=%q peer_id=%q seed_peer_present=%t approve_decl_present=%t decls_present=%t target_present=%t session_present=%t",
		keys,
		strings.TrimSpace(meta["op"]),
		strings.TrimSpace(meta["peer_id"]),
		strings.TrimSpace(meta["seed_peer"]) != "",
		strings.TrimSpace(meta["approve_decl"]) != "",
		strings.TrimSpace(meta["decls"]) != "",
		strings.TrimSpace(meta["target"]) != "",
		strings.TrimSpace(meta["session"]) != "",
	)
}

func safeAcceptorMQTTSummary(local *pocstate.LocalConfig) string {
	if local == nil {
		return "broker= data_proto= quic_cc= p2p_network= p2p_ip_family= p2p_port=0"
	}
	return fmt.Sprintf(
		"broker=%s data_proto=%s quic_cc=%s p2p_network=%s p2p_ip_family=%s p2p_port=%d",
		strings.TrimSpace(local.MQTTBroker),
		strings.TrimSpace(local.DataProto),
		strings.TrimSpace(local.QUICCC),
		strings.TrimSpace(local.P2PNetwork),
		strings.TrimSpace(local.P2PIPFamily),
		local.P2PPort,
	)
}

func serveAcceptedShellStream(
	ctx context.Context,
	stateDir string,
	local *pocstate.LocalConfig,
	locks *shelllock.Manager,
	lockTTL time.Duration,
	sess dataplane.PeerSession,
	accepted *dataplane.AcceptedStream,
	bindPeer func(string),
	reg *peerSessionRegistry,
) error {
	if accepted == nil || accepted.Stream == nil {
		return errors.New("nil accepted stream")
	}

	stream := accepted.Stream
	reader := shellproto.NewReader(stream)
	writer := shellproto.NewWriter(stream)

	closeStream := true
	defer func() {
		if closeStream {
			_ = stream.Close()
		}
	}()

	if accepted.Open.Kind != dataplane.StreamKindShellV0 {
		_ = writeHelloError(writer, shellproto.ReasonHelloInvalid, "unsupported stream kind", []string{"upgrade both ends"})
		return fmt.Errorf("unsupported stream kind: %q", accepted.Open.Kind)
	}

	helloCtl := shellproto.Control{
		Op:     shellproto.OpHello,
		PeerID: accepted.Open.Metadata["peer_id"],
		SigB64: accepted.Open.Metadata["sig_b64"],
	}
	if approveDecl := strings.TrimSpace(accepted.Open.Metadata["approve_decl"]); approveDecl != "" {
		helloCtl.ApproveDecl = json.RawMessage(approveDecl)
	}
	if declsRaw := strings.TrimSpace(accepted.Open.Metadata["decls"]); declsRaw != "" {
		_ = json.Unmarshal([]byte(declsRaw), &helloCtl.Decls)
	}
	if seedRaw := strings.TrimSpace(accepted.Open.Metadata["seed_peer"]); seedRaw != "" {
		var seed shellproto.PeerSeed
		if err := json.Unmarshal([]byte(seedRaw), &seed); err == nil {
			helloCtl.SeedPeer = &seed
		}
	}
	if err := handleHello(ctx, stateDir, writer, helloCtl); err != nil {
		if errors.Is(err, errHelloRevoked) && reg != nil {
			// Strong revoke semantics: once we observe revoke locally, cut off
			// existing sessions for that peer.
			reg.ClosePeer(strings.TrimSpace(helloCtl.PeerID), dataplane.CloseReasonAuthorizationRevocation)
		}
		return err
	}
	if bindPeer != nil {
		bindPeer(helloCtl.PeerID)
	}

	ctl, err := readShellControl(ctx, stream, reader)
	if err != nil {
		return err
	}
	if err := checkStreamOpenMatchesControl(accepted.Open, ctl); err != nil {
		_ = writeHelloError(writer, shellproto.ReasonHelloInvalid, err.Error(), []string{"retry"})
		return err
	}

	switch strings.TrimSpace(ctl.Op) {
	case shellproto.OpPing:
		return servePing(writer)
	case shellproto.OpShLS:
		return serveShLS(ctx, writer, ctl)
	case shellproto.OpShAttach:
		closeStream = false
		err := serveShAttach(ctx, local, locks, lockTTL, reader, writer, stream, ctl)
		_ = stream.Close()
		return err
	default:
		return errors.New("unknown op")
	}
}

func readShellControl(ctx context.Context, stream io.Closer, reader *shellproto.Reader) (shellproto.Control, error) {
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

func checkStreamOpenMatchesControl(open dataplane.StreamOpen, ctl shellproto.Control) error {
	metaOp := strings.TrimSpace(open.Metadata["op"])
	if metaOp != "" && strings.TrimSpace(ctl.Op) != metaOp {
		return fmt.Errorf("stream-open op %q does not match payload op %q", metaOp, strings.TrimSpace(ctl.Op))
	}
	metaTarget := strings.TrimSpace(open.Metadata["target"])
	if metaTarget != "" && strings.TrimSpace(ctl.Target) != metaTarget {
		return fmt.Errorf("stream-open target %q does not match payload target %q", metaTarget, strings.TrimSpace(ctl.Target))
	}
	metaSession := strings.TrimSpace(open.Metadata["session"])
	if metaSession != "" && strings.TrimSpace(ctl.Session) != metaSession {
		return fmt.Errorf("stream-open session %q does not match payload session %q", metaSession, strings.TrimSpace(ctl.Session))
	}
	return nil
}

var errHelloRevoked = errors.New("hello revoked")

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
	incomingDecls := append([]json.RawMessage(nil), req.Decls...)
	if len(req.ApproveDecl) > 0 {
		incomingDecls = append(incomingDecls, req.ApproveDecl)
	}
	if len(incomingDecls) > 0 {
		merged, err := pocstate.MergeVerifiedDecls(stateDir, head, incomingDecls)
		if err != nil {
			_ = writeHelloError(w, shellproto.ReasonHelloInternal, "failed to persist decls", []string{"retry"})
			return err
		}
		declsFile = merged
	}

	revoked, err := isRevokedV0(head, declsFile.Decls, peerID)
	if err != nil {
		_ = writeHelloError(w, shellproto.ReasonHelloInternal, "failed to validate local decls", []string{"retry"})
		return err
	}
	if revoked && !head.IsAdmin(peerID) && !head.IsOwner(peerID) {
		_ = writeHelloError(w, shellproto.ReasonHelloRevoked, "peer_id is revoked", []string{"re-join with a new identity"})
		return errHelloRevoked
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
	if err := persistHelloSeedPeer(stateDir, peerID, req.SeedPeer); err != nil {
		_ = writeHelloError(w, shellproto.ReasonHelloInternal, "failed to persist peer seed", []string{"retry"})
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

	return w.WriteJSON(shellproto.Control{
		Op:    shellproto.OpHello,
		OK:    true,
		Decls: pocstate.RawDeclMessages(declsFile.Decls),
	})
}

func persistHelloSeedPeer(stateDir string, peerID string, seed *shellproto.PeerSeed) error {
	if seed == nil {
		return nil
	}
	peerID, err := controlplane.CanonicalizePeerID(peerID)
	if err != nil {
		return nil
	}
	if strings.TrimSpace(seed.PeerID) != peerID {
		return nil
	}
	if strings.TrimSpace(seed.ProxyName) == "" ||
		strings.TrimSpace(seed.SecretKey) == "" ||
		strings.TrimSpace(seed.TopicPrefix) == "" {
		return nil
	}
	if len(normalizeRuntimeSeedBrokers(seed)) == 0 {
		return nil
	}

	statePath := filepath.Join(stateDir, "state.json")
	st, err := pocstate.Load(statePath)
	if err != nil {
		return err
	}
	cfg := pocstate.PeerConfig{
		ProxyName:   strings.TrimSpace(seed.ProxyName),
		SecretKey:   strings.TrimSpace(seed.SecretKey),
		TopicPrefix: strings.TrimSpace(seed.TopicPrefix),
		V4Hint:      pocstate.NormalizeV4Hint(seed.V4Hint),
		V6Hint:      pocstate.NormalizeV6Hint(seed.V6Hint),
		DataProto:   strings.TrimSpace(seed.DataProto),
		QUICCC:      strings.TrimSpace(seed.QUICCC),
	}
	cfg.SetMQTTBrokers(normalizeRuntimeSeedBrokers(seed))
	cfg.NormalizeDefaults()
	st.UpsertPeer(peerID, cfg)
	return pocstate.Save(statePath, st)
}

func normalizeRuntimeSeedBrokers(seed *shellproto.PeerSeed) []string {
	if seed == nil {
		return nil
	}
	candidates := append([]string(nil), seed.MQTTBrokers...)
	if strings.TrimSpace(seed.MQTTBroker) != "" {
		candidates = append(candidates, seed.MQTTBroker)
	}
	out := make([]string, 0, len(candidates))
	seen := make(map[string]struct{}, len(candidates))
	for _, broker := range candidates {
		broker = strings.TrimSpace(broker)
		if broker == "" {
			continue
		}
		if _, ok := seen[broker]; ok {
			continue
		}
		seen[broker] = struct{}{}
		out = append(out, broker)
	}
	return out
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

func brokerFailuresErrorForLog(failures []string, fallback string) error {
	if len(failures) == 0 {
		return errors.New(fallback)
	}
	return errors.New(strings.Join(failures, "; "))
}
