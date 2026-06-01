package task

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"
	"time"

	"github.com/apernet/quic-go"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/punchdecision"
	"github.com/miopunch/miopunch/internal/punching"
	mqttsig "github.com/miopunch/miopunch/internal/signaling/mqtt"
	"github.com/miopunch/miopunch/internal/udpowner"
	"github.com/miopunch/miopunch/internal/wire"
)

type dialResult struct {
	stream io.ReadWriteCloser
	sess   dataplane.PeerSession

	sid         string
	dataProto   string
	quicCC      string
	attemptWay  string
	legacyHello bool
	sessionLive bool
}

type ownedPeerSession struct {
	sess    dataplane.PeerSession
	closers []io.Closer
}

func (s *ownedPeerSession) Key() dataplane.SessionKey { return s.sess.Key() }
func (s *ownedPeerSession) OpenStream(ctx context.Context, open dataplane.StreamOpen) (io.ReadWriteCloser, error) {
	return s.sess.OpenStream(ctx, open)
}
func (s *ownedPeerSession) AcceptStream(ctx context.Context) (*dataplane.AcceptedStream, error) {
	return s.sess.AcceptStream(ctx)
}
func (s *ownedPeerSession) Close(reason dataplane.CloseReason) error {
	var firstErr error
	if err := s.sess.Close(reason); err != nil {
		firstErr = err
	}
	for _, c := range s.closers {
		if c == nil {
			continue
		}
		if err := c.Close(); err != nil && firstErr == nil {
			firstErr = err
		}
	}
	return firstErr
}
func (s *ownedPeerSession) CloseReason() dataplane.CloseReason { return s.sess.CloseReason() }
func (s *ownedPeerSession) Healthy() bool                      { return s.sess.Healthy() }
func (s *ownedPeerSession) LastActivity() time.Time            { return s.sess.LastActivity() }
func (s *ownedPeerSession) SessionPathFacts() dataplane.SessionPathFacts {
	return dataplane.PathFactsFromSession(s.sess)
}

type pathFactsPeerSession struct {
	dataplane.PeerSession
	facts dataplane.SessionPathFacts
}

func (s *pathFactsPeerSession) SessionPathFacts() dataplane.SessionPathFacts {
	if s == nil {
		return dataplane.SessionPathFacts{}
	}
	return dataplane.PathFactsFromSession(s.PeerSession).Merge(s.facts)
}

func withSessionPathFacts(sess dataplane.PeerSession, facts dataplane.SessionPathFacts) dataplane.PeerSession {
	if sess == nil || facts.Empty() {
		return sess
	}
	return &pathFactsPeerSession{PeerSession: sess, facts: facts}
}

func sessionPathFactsForAttempt(attemptPath string) dataplane.SessionPathFacts {
	return dataplane.SessionPathFacts{PunchStatus: punchStatusFromAttemptPath(attemptPath)}
}

func punchStatusFromAttemptPath(attemptPath string) string {
	attemptPath = strings.TrimSpace(attemptPath)
	switch {
	case attemptPath == "":
		return ""
	case strings.HasPrefix(attemptPath, "punching_"):
		return "punching"
	case strings.HasPrefix(attemptPath, "direct_"):
		return "direct"
	case strings.HasPrefix(attemptPath, "passive_accept_"):
		return attemptPath
	case attemptPath == "session_reuse":
		return "session_reuse"
	default:
		return ""
	}
}

func selectedTopologyView(res *punchdecision.Result, attemptPath string) string {
	view, _ := selectedTopologyViewReason(res, attemptPath)
	return view
}

func selectedTopologyReason(res *punchdecision.Result, attemptPath string) string {
	_, reason := selectedTopologyViewReason(res, attemptPath)
	return reason
}

func selectedTopologyViewReason(res *punchdecision.Result, attemptPath string) (string, string) {
	if res == nil {
		return "", ""
	}
	viewValue := func(r *wire.NatHoleResp) string {
		return r.SelectedView
	}
	reasonValue := func(r *wire.NatHoleResp) string {
		return r.SelectedReason
	}
	if strings.Contains(strings.TrimSpace(attemptPath), "tcp") {
		viewValue = func(r *wire.NatHoleResp) string {
			return r.TCPSelectedView
		}
		reasonValue = func(r *wire.NatHoleResp) string {
			return r.TCPSelectedReason
		}
	}
	return firstNatHoleRespText(res.VisitorResponse, res.ClientResponse, viewValue),
		firstNatHoleRespText(res.VisitorResponse, res.ClientResponse, reasonValue)
}

func firstNatHoleRespText(a *wire.NatHoleResp, b *wire.NatHoleResp, value func(*wire.NatHoleResp) string) string {
	if value == nil {
		return ""
	}
	if a != nil {
		if out := strings.TrimSpace(value(a)); out != "" {
			return out
		}
	}
	if b != nil {
		return strings.TrimSpace(value(b))
	}
	return ""
}

func (m *Manager) dialPeerStream(ctx context.Context, taskID string, peerID string, cfg pocstate.PeerConfig, open dataplane.StreamOpen) (*dialResult, error) {
	if m != nil {
		m.mu.Lock()
		hook := m.dialPeerStreamHook
		m.mu.Unlock()
		if hook != nil {
			stream, err := hook(ctx, taskID, peerID, cfg)
			if err != nil {
				return nil, err
			}
			m.addFact(taskID, poc.Fact{TermID: "peer_id", Message: "peer_id=" + peerID})
			return &dialResult{stream: stream, legacyHello: true}, nil
		}
	}

	startedAt := time.Now().UTC().UnixMilli()
	cfg.NormalizeDefaults()

	if strings.TrimSpace(cfg.ProxyName) == "" {
		return nil, errors.New("missing proxy_name in peer config")
	}
	if strings.TrimSpace(cfg.SecretKey) == "" {
		return nil, errors.New("missing secret_key in peer config")
	}

	sid := mqttsig.DeriveSID(cfg.ProxyName, cfg.SecretKey)
	if open.Kind == "" {
		open.Kind = dataplane.StreamKindShellV0
	}

	if m != nil && m.sessions != nil {
		sess, ok := findReusableSession(m.sessions, peerID, sid, cfg)
		if ok {
			stream, err := sess.OpenStream(ctx, open)
			if err == nil {
				key := sess.Key()
				m.addFact(taskID, poc.Fact{TermID: "peer_id", Message: "peer_id=" + peerID})
				m.addFact(taskID, poc.Fact{TermID: "sid", Message: "sid=" + sid})
				m.addFact(taskID, poc.Fact{TermID: "data_proto", Message: "data_proto=" + string(key.Protocol)})
				m.addFact(taskID, poc.Fact{TermID: "session_reused", Message: "session_reused=true"})
				if key.PathFamily != "" {
					m.addFact(taskID, poc.Fact{TermID: "path_family", Message: "path_family=" + string(key.PathFamily)})
				}
				m.recordTopologyAttempt(TopologyAttempt{
					PeerID:      peerID,
					AttemptPath: "session_reuse",
					AttemptWay:  "session_reuse",
					DataProto:   string(key.Protocol),
					PathFamily:  string(key.PathFamily),
					StartedAt:   startedAt,
					Outcome:     "ok",
				})
				return &dialResult{
					stream:      stream,
					sess:        sess,
					sid:         sid,
					dataProto:   string(key.Protocol),
					attemptWay:  "session_reuse",
					sessionLive: true,
				}, nil
			}
			m.sessions.Close(sess.Key(), dataplane.CloseReasonTransportFatal)
		}
	}

	var diagEvents bytes.Buffer
	diagEmitter := event.NewEmitter(&diagEvents, "task")

	m.setStage(taskID, poc.StageCandidateExchange, "gather candidates")
	gather, err := connectivity.Gather(ctx, sid, connectivity.GatherConfig{
		ListenPort:           cfg.P2PPort,
		P2PNetwork:           connectivity.P2PNetwork(cfg.P2PNetwork),
		P2PIPFamily:          connectivity.P2PIPFamily(cfg.P2PIPFamily),
		DisableAssistedAddrs: cfg.DisableAssistedAddrs,
		DisablePortMap:       cfg.DisablePortMap,
		StunServers:          cfg.StunServers,
		StunExplicit:         cfg.StunExplicit,
		Emitter:              diagEmitter,
	})
	if err != nil {
		return nil, err
	}
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

	transactionID := punching.NewTransactionID()

	var brutalUpBps, brutalDownBps uint64
	if cfg.DataProto == "quic" && cfg.QUICCC == "brutal" {
		brutalUpBps = 1_000_000
		brutalDownBps = 1_000_000
	}

	natHoleVisitorMsg := &wire.NatHoleVisitor{
		TransactionID:    transactionID,
		ProxyName:        cfg.ProxyName,
		Protocol:         cfg.DataProto,
		QuicCC:           cfg.QUICCC,
		BrutalUpBps:      brutalUpBps,
		BrutalDownBps:    brutalDownBps,
		Capabilities:     []string{wire.CapabilityTCPP2PV0},
		P2PNetwork:       cfg.P2PNetwork,
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

	m.setStage(taskID, poc.StageCandidateExchange, "mqtt exchange")
	runtimeBrokers := runtimeBrokerEndpointsForPeer(cfg)
	if len(runtimeBrokers) == 0 {
		return nil, errors.New("missing mqtt_broker in peer config")
	}

	runVisitorExchange := func(visitorMsg *wire.NatHoleVisitor) (*wire.NatHoleResp, *punchdecision.Result, error) {
		var (
			exchangeResp     *wire.NatHoleResp
			exchangeDecision *punchdecision.Result
			mqttFailures     []string
		)
		for _, broker := range runtimeBrokers {
			mq, openErr := mqttsig.Open(ctx, mqttsig.Config{
				BrokerURL:       mqttBrokerURL(broker),
				TopicPrefix:     cfg.TopicPrefix,
				SID:             sid,
				Role:            mqttsig.RoleVisitor,
				HelloTimeout:    10 * time.Second,
				ExchangeTimeout: 10 * time.Second,
				BarrierTimeout:  10 * time.Second,
			})
			if openErr != nil {
				mqttFailures = append(mqttFailures, fmt.Sprintf("%s: %v", broker, openErr))
				m.addFact(taskID, poc.Fact{Message: "mqtt broker skipped: " + broker + ": " + openErr.Error()})
				continue
			}

			var runErr error
			exchangeResp, runErr = mq.RunVisitor(ctx, visitorMsg, func(sid string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*wire.NatHoleResp, *wire.NatHoleResp, error) {
				res, err := punchdecision.AnalyzeWithDaemonMemory(sid, peerID, visitor, client)
				if err != nil {
					return nil, nil, err
				}
				exchangeDecision = res
				return res.VisitorResponse, res.ClientResponse, nil
			})
			_ = mq.Close()
			if runErr == nil {
				return exchangeResp, exchangeDecision, nil
			}
			if ctx.Err() != nil {
				return nil, nil, ctx.Err()
			}
			mqttFailures = append(mqttFailures, fmt.Sprintf("%s: %v", broker, runErr))
			m.addFact(taskID, poc.Fact{Message: "mqtt broker skipped: " + broker + ": " + runErr.Error()})
		}
		return nil, nil, brokerFailuresError(mqttFailures, "mqtt exchange failed on all effective brokers")
	}

	natHoleRespMsg, decisionRes, err := runVisitorExchange(natHoleVisitorMsg)
	if err != nil {
		return nil, err
	}

	if natHoleRespMsg != nil {
		db := natHoleRespMsg.DetectBehavior
		m.addFact(taskID, poc.Fact{TermID: "punching_plan", Message: fmt.Sprintf(
			"punching_plan: enabled=%t mode=%d role=%s ttl=%d send_delay_ms=%d read_timeout_ms=%d candidate_addrs=%d assisted_addrs=%d candidate_ports=%d send_random_ports=%d listen_random_ports=%d",
			natHoleRespMsg.PunchingEnabled,
			db.Mode,
			db.Role,
			db.TTL,
			db.SendDelayMs,
			db.ReadTimeoutMs,
			len(natHoleRespMsg.CandidateAddrs),
			len(natHoleRespMsg.AssistedAddrs),
			len(db.CandidatePorts),
			db.SendRandomPorts,
			db.ListenRandomPorts,
		)})

		if natHoleRespMsg.TCPDetectBehavior != nil {
			tdb := natHoleRespMsg.TCPDetectBehavior
			m.addFact(taskID, poc.Fact{TermID: "tcp_punching_plan", Message: fmt.Sprintf(
				"tcp_punching_plan: enabled=%t mode=%d role=%s send_delay_ms=%d read_timeout_ms=%d candidate_ports=%d send_random_ports=%d listen_random_ports=%d",
				natHoleRespMsg.TCPPunchingEnabled,
				tdb.Mode,
				tdb.Role,
				tdb.SendDelayMs,
				tdb.ReadTimeoutMs,
				len(tdb.CandidatePorts),
				tdb.SendRandomPorts,
				tdb.ListenRandomPorts,
			)})
		}
	}

	if decisionRes != nil && decisionRes.ClientResponse != nil {
		peer := decisionRes.ClientResponse
		db := peer.DetectBehavior
		m.addFact(taskID, poc.Fact{TermID: "peer_punching_plan", Message: fmt.Sprintf(
			"peer_punching_plan: enabled=%t mode=%d role=%s ttl=%d send_delay_ms=%d read_timeout_ms=%d candidate_addrs=%d assisted_addrs=%d candidate_ports=%d send_random_ports=%d listen_random_ports=%d",
			peer.PunchingEnabled,
			db.Mode,
			db.Role,
			db.TTL,
			db.SendDelayMs,
			db.ReadTimeoutMs,
			len(peer.CandidateAddrs),
			len(peer.AssistedAddrs),
			len(db.CandidatePorts),
			db.SendRandomPorts,
			db.ListenRandomPorts,
		)})

		if peer.TCPDetectBehavior != nil {
			tdb := peer.TCPDetectBehavior
			m.addFact(taskID, poc.Fact{TermID: "peer_tcp_punching_plan", Message: fmt.Sprintf(
				"peer_tcp_punching_plan: enabled=%t mode=%d role=%s send_delay_ms=%d read_timeout_ms=%d candidate_ports=%d send_random_ports=%d listen_random_ports=%d",
				peer.TCPPunchingEnabled,
				tdb.Mode,
				tdb.Role,
				tdb.SendDelayMs,
				tdb.ReadTimeoutMs,
				len(tdb.CandidatePorts),
				tdb.SendRandomPorts,
				tdb.ListenRandomPorts,
			)})
		}
	}

	// UDP socket owner / demux wiring (for UDP dataplane protocols).
	// Traversal and dataplane MUST share the same UDP socket mapping.
	var (
		udp4Demux *udpowner.TraversalDemux
		udp6Demux *udpowner.TraversalDemux

		quicUDP4Transport *quic.Transport
		quicUDP6Transport *quic.Transport

		kcpUDP4Owner *udpowner.KCPOwner
		kcpUDP6Owner *udpowner.KCPOwner
	)
	defer func() {
		// If the session ends up owning these resources, they will be moved into
		// an ownedPeerSession and shouldn't be closed here.
		if quicUDP4Transport != nil {
			_ = quicUDP4Transport.Close()
		}
		if quicUDP6Transport != nil {
			_ = quicUDP6Transport.Close()
		}
		// For KCP mode, traversal demux is owned by the KCPOwner.
		if udp4Demux != nil && kcpUDP4Owner == nil {
			_ = udp4Demux.Close()
		}
		if udp6Demux != nil && kcpUDP6Owner == nil {
			_ = udp6Demux.Close()
		}
		if kcpUDP4Owner != nil {
			_ = kcpUDP4Owner.Close()
		}
		if kcpUDP6Owner != nil {
			_ = kcpUDP6Owner.Close()
		}
	}()

	keyBytes := []byte(cfg.SecretKey)
	if natHoleRespMsg != nil {
		switch dataplane.Protocol(natHoleRespMsg.Protocol) {
		case dataplane.ProtocolQUIC:
			if gather.UDP4Conn != nil {
				tr := &quic.Transport{Conn: gather.UDP4Conn}
				d, err := udpowner.NewQUICTraversalDemux(tr, udpowner.DemuxConfig{Key: keyBytes})
				if err != nil {
					return nil, err
				}
				quicUDP4Transport = tr
				udp4Demux = d
			}
			if gather.UDP6Conn != nil {
				tr := &quic.Transport{Conn: gather.UDP6Conn}
				d, err := udpowner.NewQUICTraversalDemux(tr, udpowner.DemuxConfig{Key: keyBytes})
				if err != nil {
					return nil, err
				}
				quicUDP6Transport = tr
				udp6Demux = d
			}
		case dataplane.ProtocolKCP:
			if gather.UDP4Conn != nil {
				o, err := udpowner.NewKCPOwner(gather.UDP4Conn, udpowner.KCPOwnerConfig{
					Traversal: udpowner.DemuxConfig{Key: keyBytes},
				})
				if err != nil {
					return nil, err
				}
				kcpUDP4Owner = o
				udp4Demux = o.TraversalDemux()
			}
			if gather.UDP6Conn != nil {
				o, err := udpowner.NewKCPOwner(gather.UDP6Conn, udpowner.KCPOwnerConfig{
					Traversal: udpowner.DemuxConfig{Key: keyBytes},
				})
				if err != nil {
					return nil, err
				}
				kcpUDP6Owner = o
				udp6Demux = o.TraversalDemux()
			}
		}
	}

	m.setStage(taskID, poc.StagePunchAttempt, "punch attempt")
	attemptCfg := connectivity.AttemptConfig{
		P2PNetwork:         connectivity.P2PNetwork(cfg.P2PNetwork),
		P2PIPFamily:        connectivity.P2PIPFamily(cfg.P2PIPFamily),
		Emitter:            diagEmitter,
		UDP4TraversalDemux: udp4Demux,
		UDP6TraversalDemux: udp6Demux,
	}
	attemptRes, err := connectivity.Attempt(ctx, sid, []byte(cfg.SecretKey), gather.UDP4Conn, gather.UDP6Conn, gather.TCP4Listener, gather.TCP6Listener, natHoleRespMsg, attemptCfg)
	if err != nil {
		recordAttemptDiagnostics(m, taskID, diagEvents.String())
		return nil, err
	}

	type sessionDialResult struct {
		sess      dataplane.PeerSession
		dpCfg     dataplane.Config
		dataProto string
		quicCC    string
	}
	dialAttemptSession := func(attemptRes *connectivity.AttemptResult) (sessionDialResult, error) {
		if attemptRes == nil {
			return sessionDialResult{}, errors.New("nil attempt result")
		}

		attemptUDP4 := attemptRes.Conn != nil && attemptRes.Conn == gather.UDP4Conn
		attemptUDP6 := attemptRes.Conn != nil && attemptRes.Conn == gather.UDP6Conn
		dpCfg := dataplane.Config{
			Proto:        dataplane.Protocol(natHoleRespMsg.Protocol),
			QuicCC:       dataplane.QUICCC(natHoleRespMsg.QuicCC),
			RemotePeerID: peerID,
			SecurityID:   sid,
			SecretKey:    []byte(cfg.SecretKey),
			PathFamily:   dataplane.PathFamilyFromAttemptPath(attemptRes.Path),
			Brutal: dataplane.BrutalConfig{
				UpBps:   natHoleRespMsg.BrutalUpBps,
				DownBps: natHoleRespMsg.BrutalDownBps,
			},
		}

		result := sessionDialResult{
			dpCfg:     dpCfg,
			dataProto: natHoleRespMsg.Protocol,
			quicCC:    natHoleRespMsg.QuicCC,
		}
		var sess dataplane.PeerSession
		var err error
		if len(attemptRes.TCPConns) > 0 {
			dpCfg.Proto = dataplane.ProtocolTLS
			result.dpCfg = dpCfg
			result.dataProto = string(dataplane.ProtocolTLS)
			result.quicCC = ""
			sess, err = dataplane.DialTLSSession(ctx, dpCfg, attemptRes.TCPConns, nil)
			if err != nil {
				return sessionDialResult{}, err
			}
			result.sess = sess
			return result, nil
		}

		switch dpCfg.Proto {
		case dataplane.ProtocolQUIC:
			var tr *quic.Transport
			var demux *udpowner.TraversalDemux
			switch {
			case attemptUDP4:
				tr, demux = quicUDP4Transport, udp4Demux
			case attemptUDP6:
				tr, demux = quicUDP6Transport, udp6Demux
			}
			if tr == nil || attemptRes.Remote == nil {
				sess, err = dataplane.DialSession(ctx, dpCfg, attemptRes.Conn, attemptRes.Remote, nil)
				break
			}
			sess, err = dataplane.DialSessionWithQUICTransport(ctx, dpCfg, tr, attemptRes.Remote, nil)
			if err == nil && sess != nil {
				// Move ownership of the UDP conn / transport / demux to the session wrapper.
				closers := []io.Closer{demux, tr, attemptRes.Conn}
				sess = &ownedPeerSession{sess: sess, closers: closers}
				if attemptUDP4 {
					gather.UDP4Conn, quicUDP4Transport, udp4Demux = nil, nil, nil
				}
				if attemptUDP6 {
					gather.UDP6Conn, quicUDP6Transport, udp6Demux = nil, nil, nil
				}
			}
		case dataplane.ProtocolKCP:
			var o *udpowner.KCPOwner
			switch {
			case attemptUDP4:
				o = kcpUDP4Owner
			case attemptUDP6:
				o = kcpUDP6Owner
			}
			if o == nil || attemptRes.Remote == nil {
				sess, err = dataplane.DialSession(ctx, dpCfg, attemptRes.Conn, attemptRes.Remote, nil)
				break
			}
			sess, err = dataplane.DialSessionWithKCPPacketConn(ctx, dpCfg, o.PacketConn(), attemptRes.Remote, nil)
			if err == nil && sess != nil {
				sess = &ownedPeerSession{sess: sess, closers: []io.Closer{o}}
				if attemptUDP4 {
					gather.UDP4Conn, kcpUDP4Owner, udp4Demux = nil, nil, nil
				}
				if attemptUDP6 {
					gather.UDP6Conn, kcpUDP6Owner, udp6Demux = nil, nil, nil
				}
			}
		default:
			sess, err = dataplane.DialSession(ctx, dpCfg, attemptRes.Conn, attemptRes.Remote, nil)
		}
		if err != nil {
			return sessionDialResult{}, err
		}
		result.sess = withSessionPathFacts(sess, sessionPathFactsForAttempt(attemptRes.Path))
		return result, nil
	}

	m.setStage(taskID, poc.StageDataplaneHandshake, "data plane dial stream")
	dialed, err := dialAttemptSession(attemptRes)
	if err != nil && shouldFallbackToUDPAfterTCPDataplaneError(cfg, attemptRes) {
		m.addFact(taskID, poc.Fact{Message: "auto_fallback=udp_after_tcp_dataplane_error"})
		m.addFact(taskID, poc.Fact{Message: "auto_fallback_tcp_error=" + err.Error()})
		fallbackVisitor := *natHoleVisitorMsg
		fallbackVisitor.TransactionID = punching.NewTransactionID()
		fallbackVisitor.P2PNetwork = string(connectivity.P2PNetworkUDPOnly)
		m.setStage(taskID, poc.StageCandidateExchange, "udp fallback exchange after tcp dataplane error")
		fallbackResp, fallbackDecision, fallbackExchangeErr := runVisitorExchange(&fallbackVisitor)
		if fallbackExchangeErr != nil {
			return nil, fmt.Errorf("tcp dataplane failed: %w; udp fallback exchange failed: %v", err, fallbackExchangeErr)
		}
		if fallbackResp == nil {
			return nil, fmt.Errorf("tcp dataplane failed: %w; udp fallback exchange returned nil response", err)
		}
		fallbackResp.P2PNetwork = string(connectivity.P2PNetworkUDPOnly)
		fallbackCfg := attemptCfg
		fallbackCfg.P2PNetwork = connectivity.P2PNetworkUDPOnly
		m.setStage(taskID, poc.StagePunchAttempt, "udp fallback after tcp dataplane error")
		fallbackRes, fallbackErr := connectivity.Attempt(ctx, sid, []byte(cfg.SecretKey), gather.UDP4Conn, gather.UDP6Conn, gather.TCP4Listener, gather.TCP6Listener, fallbackResp, fallbackCfg)
		if fallbackErr != nil {
			return nil, fmt.Errorf("tcp dataplane failed: %w; udp fallback failed: %v", err, fallbackErr)
		}
		natHoleRespMsg = fallbackResp
		decisionRes = fallbackDecision
		attemptRes = fallbackRes
		m.setStage(taskID, poc.StageDataplaneHandshake, "data plane dial stream")
		dialed, err = dialAttemptSession(attemptRes)
	}
	if err != nil {
		return nil, err
	}
	sess := dialed.sess
	dpCfg := dialed.dpCfg
	switch attemptRes.Path {
	case "punching_ipv4":
		punchdecision.ReportDaemonUDPSuccess(decisionRes)
	case "punching_tcp4":
		punchdecision.ReportDaemonTCPSuccess(decisionRes)
	}
	stream, err := sess.OpenStream(ctx, open)
	if err != nil {
		_ = sess.Close(dataplane.CloseReasonTransportFatal)
		return nil, err
	}

	m.addFact(taskID, poc.Fact{TermID: "peer_id", Message: "peer_id=" + peerID})
	m.addFact(taskID, poc.Fact{TermID: "sid", Message: "sid=" + sid})
	dataProto := dialed.dataProto
	quicCC := dialed.quicCC

	m.addFact(taskID, poc.Fact{TermID: "data_proto", Message: "data_proto=" + dataProto})
	if strings.TrimSpace(quicCC) != "" {
		m.addFact(taskID, poc.Fact{TermID: "quic_cc", Message: "quic_cc=" + quicCC})
	}
	if strings.TrimSpace(attemptRes.Path) != "" {
		m.addFact(taskID, poc.Fact{TermID: "attempt_path", Message: "attempt_path=" + attemptRes.Path})
	}
	if dpCfg.PathFamily != "" {
		m.addFact(taskID, poc.Fact{TermID: "path_family", Message: "path_family=" + string(dpCfg.PathFamily)})
	}
	m.recordTopologyAttempt(TopologyAttempt{
		PeerID:         peerID,
		AttemptPath:    attemptRes.Path,
		AttemptWay:     attemptRes.Path,
		DataProto:      dataProto,
		PathFamily:     string(dpCfg.PathFamily),
		Portmap:        topologyPortmapEvidenceFromEvents(diagEvents.String()),
		SelectedView:   selectedTopologyView(decisionRes, attemptRes.Path),
		SelectedReason: selectedTopologyReason(decisionRes, attemptRes.Path),
		StartedAt:      startedAt,
		Outcome:        "ok",
	})

	return &dialResult{
		stream:     stream,
		sess:       sess,
		sid:        sid,
		dataProto:  dataProto,
		quicCC:     quicCC,
		attemptWay: attemptRes.Path,
	}, nil
}

func (m *Manager) markDialedSessionLive(res *dialResult) {
	if m == nil || m.sessions == nil || res == nil || res.sess == nil || res.sessionLive {
		return
	}
	m.sessions.Put(res.sess)
	res.sessionLive = true
}

func (m *Manager) closeDialedSession(res *dialResult, reason dataplane.CloseReason) {
	if res == nil || res.sess == nil {
		return
	}
	if res.sessionLive {
		if m != nil && m.sessions != nil {
			m.sessions.Close(res.sess.Key(), reason)
			res.sessionLive = false
			return
		}
	}
	_ = res.sess.Close(reason)
	res.sessionLive = false
}

func (m *Manager) recordDialedSessionFailure(peerID string, res *dialResult, stage poc.Stage, reasonCode poc.ReasonCode, stopCondition string) {
	if m == nil {
		return
	}
	attempt := TopologyAttempt{
		PeerID:        strings.TrimSpace(peerID),
		AttemptPath:   "unknown",
		AttemptWay:    "unknown",
		StartedAt:     time.Now().UTC().UnixMilli(),
		Outcome:       "fail",
		Stage:         string(stage),
		ReasonCode:    string(reasonCode),
		StopCondition: strings.TrimSpace(stopCondition),
	}
	if res != nil {
		if strings.TrimSpace(res.attemptWay) != "" {
			attempt.AttemptPath = res.attemptWay
			attempt.AttemptWay = res.attemptWay
		}
		attempt.DataProto = strings.TrimSpace(res.dataProto)
		if res.sess != nil {
			key := res.sess.Key().Normalize()
			if attempt.PeerID == "" {
				attempt.PeerID = key.RemotePeerID
			}
			if attempt.DataProto == "" {
				attempt.DataProto = string(key.Protocol)
			}
			if key.PathFamily != "" {
				attempt.PathFamily = string(key.PathFamily)
			}
		}
	}
	m.recordTopologyAttempt(attempt)
}

func (m *Manager) loadPeerConfig(peerID string) (pocstate.PeerConfig, bool, error) {
	st, err := m.loadState()
	if err != nil {
		return pocstate.PeerConfig{}, false, err
	}
	cfg, ok := st.Peers[strings.TrimSpace(peerID)]
	if !ok {
		return pocstate.PeerConfig{}, false, nil
	}
	if st.Local != nil {
		local := *st.Local
		local.NormalizeDefaults()
		cfg = peerConfigWithLocalDialDefaults(cfg, local)
	}
	cfg.NormalizeDefaults()
	return cfg, true, nil
}

func recordAttemptDiagnostics(m *Manager, taskID string, raw string) {
	if m == nil {
		return
	}
	for _, line := range strings.Split(raw, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		var ev event.Event
		if err := json.Unmarshal([]byte(line), &ev); err != nil {
			continue
		}
		if !strings.HasPrefix(ev.Name, "attempt.punching.") && !strings.HasPrefix(ev.Name, "attempt.tcp_punching.") {
			continue
		}

		errStr := ""
		if strings.TrimSpace(ev.Err) != "" {
			errStr = " err=" + strings.TrimSpace(ev.Err)
		}
		kvsStr := ""
		if len(ev.KVs) > 0 {
			if b, err := json.Marshal(ev.KVs); err == nil {
				kvsStr = " kvs=" + string(b)
			}
		}

		msg := strings.TrimSpace(ev.Msg)
		if msg == "" {
			msg = "-"
		}
		m.addFact(taskID, poc.Fact{Message: "attempt_diag: " + ev.Name + " msg=" + msg + errStr + kvsStr})
	}
}

func shouldFallbackToUDPAfterTCPDataplaneError(cfg pocstate.PeerConfig, attemptRes *connectivity.AttemptResult) bool {
	if attemptRes == nil || len(attemptRes.TCPConns) == 0 {
		return false
	}
	network, err := connectivity.ParseP2PNetwork(cfg.P2PNetwork)
	if err != nil {
		return false
	}
	return network == connectivity.P2PNetworkAuto
}

func findReusableSession(sessions *dataplane.SessionManager, peerID string, sid string, cfg pocstate.PeerConfig) (dataplane.PeerSession, bool) {
	if sessions == nil {
		return nil, false
	}
	for _, protocol := range reusableSessionProtocols(cfg) {
		for _, pathFamily := range reusableSessionPathFamilies(cfg.P2PNetwork) {
			key := dataplane.SessionKey{
				RemotePeerID: peerID,
				Protocol:     protocol,
				SecurityID:   sid,
				PathFamily:   pathFamily,
			}
			if sess, ok := sessions.Find(key); ok {
				return sess, true
			}
		}
	}
	return nil, false
}

func reusableSessionProtocols(cfg pocstate.PeerConfig) []dataplane.Protocol {
	switch connectivity.P2PNetwork(strings.TrimSpace(cfg.P2PNetwork)) {
	case connectivity.P2PNetworkTCPOnly:
		return []dataplane.Protocol{dataplane.ProtocolTLS}
	case connectivity.P2PNetworkUDPOnly:
		return []dataplane.Protocol{dataplane.Protocol(strings.TrimSpace(cfg.DataProto))}
	default:
		protocol := dataplane.Protocol(strings.TrimSpace(cfg.DataProto))
		if protocol == "" || protocol == dataplane.ProtocolTLS {
			return []dataplane.Protocol{dataplane.ProtocolTLS}
		}
		return []dataplane.Protocol{protocol, dataplane.ProtocolTLS}
	}
}

func reusableSessionPathFamilies(p2pNetwork string) []dataplane.PathFamily {
	switch connectivity.P2PNetwork(strings.TrimSpace(p2pNetwork)) {
	case connectivity.P2PNetworkTCPOnly:
		return []dataplane.PathFamily{dataplane.PathFamilyTCP6, dataplane.PathFamilyTCP4}
	case connectivity.P2PNetworkUDPOnly:
		return []dataplane.PathFamily{dataplane.PathFamilyUDP6, dataplane.PathFamilyUDP4}
	default:
		return []dataplane.PathFamily{dataplane.PathFamilyUnknown}
	}
}

func peerConfigWithLocalDialDefaults(cfg pocstate.PeerConfig, local pocstate.LocalConfig) pocstate.PeerConfig {
	if strings.TrimSpace(cfg.P2PNetwork) == "" && strings.TrimSpace(local.P2PNetwork) != "" {
		cfg.P2PNetwork = local.P2PNetwork
	}
	if strings.TrimSpace(cfg.P2PIPFamily) == "" && strings.TrimSpace(local.P2PIPFamily) != "" {
		cfg.P2PIPFamily = local.P2PIPFamily
	}
	if len(cfg.StunServers) == 0 && len(local.StunServers) > 0 {
		cfg.StunServers = append([]string(nil), local.StunServers...)
	}
	if !cfg.StunExplicit && local.StunExplicit {
		cfg.StunExplicit = true
	}
	cfg.DisablePortMap = cfg.DisablePortMap || local.DisablePortMap
	cfg.DisableAssistedAddrs = cfg.DisableAssistedAddrs || local.DisableAssistedAddrs
	return cfg
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

func remoteReasonToPOC(reason string) (poc.ReasonCode, poc.ExitCode) {
	switch strings.TrimSpace(reason) {
	case "HELLO_REQUIRED":
		return poc.ReasonCodeForbidden, poc.ExitCodeForbidden
	case "HELLO_INVALID":
		return poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest
	case "HELLO_NOT_APPROVED":
		return poc.ReasonCodeForbidden, poc.ExitCodeForbidden
	case "HELLO_REVOKED":
		return poc.ReasonCodeForbidden, poc.ExitCodeForbidden
	case "HELLO_ISSUER_NOT_ADMIN":
		return poc.ReasonCodeForbidden, poc.ExitCodeForbidden
	case "HELLO_DECL_INVALID":
		return poc.ReasonCodeForbidden, poc.ExitCodeForbidden
	case "HELLO_SIG_INVALID":
		return poc.ReasonCodeForbidden, poc.ExitCodeForbidden
	case "HELLO_INTERNAL":
		return poc.ReasonCodeInternal, poc.ExitCodeInternal
	case "SH_TARGET_NOT_FOUND":
		return poc.ReasonCodeSHTargetNotFound, poc.ExitCodeNotFound
	case "SH_TARGET_AMBIGUOUS":
		return poc.ReasonCodeSHTargetAmbiguous, poc.ExitCodeBadRequest
	case "SH_IN_USE":
		return poc.ReasonCodeSHInUse, poc.ExitCodeConflict
	case "SH_CONNECTOR_FAIL":
		return poc.ReasonCodeSHConnectorFail, poc.ExitCodeUnavailable
	case "SH_TMUX_MISSING":
		return poc.ReasonCodeSHTmuxMissing, poc.ExitCodeUnavailable
	case "SH_TMUX_ATTACH_FAIL":
		return poc.ReasonCodeSHTmuxAttachFail, poc.ExitCodeUnavailable
	default:
		return poc.ReasonCodeUnavailable, poc.ExitCodeUnavailable
	}
}
