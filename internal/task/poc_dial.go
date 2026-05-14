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

	sid         string
	dataProto   string
	quicCC      string
	attemptWay  string
	legacyHello bool
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
					stream:     stream,
					sid:        sid,
					dataProto:  string(key.Protocol),
					attemptWay: "session_reuse",
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
	var decisionRes *punchdecision.Result
	runtimeBrokers := runtimeBrokerEndpointsForPeer(cfg)
	if len(runtimeBrokers) == 0 {
		return nil, errors.New("missing mqtt_broker in peer config")
	}

	var (
		natHoleRespMsg *wire.NatHoleResp
		mqttFailures   []string
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

		natHoleRespMsg, err = mq.RunVisitor(ctx, natHoleVisitorMsg, func(sid string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*wire.NatHoleResp, *wire.NatHoleResp, error) {
			res, err := punchdecision.AnalyzeWithDaemonMemory(sid, peerID, visitor, client)
			if err != nil {
				return nil, nil, err
			}
			decisionRes = res
			return res.VisitorResponse, res.ClientResponse, nil
		})
		_ = mq.Close()
		if err == nil {
			break
		}
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		mqttFailures = append(mqttFailures, fmt.Sprintf("%s: %v", broker, err))
		m.addFact(taskID, poc.Fact{Message: "mqtt broker skipped: " + broker + ": " + err.Error()})
	}
	if err != nil {
		return nil, brokerFailuresError(mqttFailures, "mqtt exchange failed on all effective brokers")
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
	attemptRes, err := connectivity.Attempt(ctx, sid, []byte(cfg.SecretKey), gather.UDP4Conn, gather.UDP6Conn, gather.TCP4Listener, gather.TCP6Listener, natHoleRespMsg, connectivity.AttemptConfig{
		P2PNetwork:         connectivity.P2PNetwork(cfg.P2PNetwork),
		P2PIPFamily:        connectivity.P2PIPFamily(cfg.P2PIPFamily),
		Emitter:            diagEmitter,
		UDP4TraversalDemux: udp4Demux,
		UDP6TraversalDemux: udp6Demux,
	})
	if err != nil {
		for _, line := range strings.Split(diagEvents.String(), "\n") {
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
		return nil, err
	}
	switch attemptRes.Path {
	case "punching_ipv4":
		punchdecision.ReportDaemonUDPSuccess(decisionRes)
	case "punching_tcp4":
		punchdecision.ReportDaemonTCPSuccess(decisionRes)
	}

	attemptUDP4 := attemptRes.Conn != nil && attemptRes.Conn == gather.UDP4Conn
	attemptUDP6 := attemptRes.Conn != nil && attemptRes.Conn == gather.UDP6Conn
	if attemptUDP4 {
		gather.UDP4Conn = nil
	}
	if attemptUDP6 {
		gather.UDP6Conn = nil
	}

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

	m.setStage(taskID, poc.StageDataplaneHandshake, "data plane dial stream")
	var sess dataplane.PeerSession
	if len(attemptRes.TCPConns) > 0 {
		dpCfg.Proto = dataplane.ProtocolTLS
		sess, err = dataplane.DialTLSSession(ctx, dpCfg, attemptRes.TCPConns, nil)
	} else {
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
					quicUDP4Transport, udp4Demux = nil, nil
				}
				if attemptUDP6 {
					quicUDP6Transport, udp6Demux = nil, nil
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
					kcpUDP4Owner, udp4Demux = nil, nil
				}
				if attemptUDP6 {
					kcpUDP6Owner, udp6Demux = nil, nil
				}
			}
		default:
			sess, err = dataplane.DialSession(ctx, dpCfg, attemptRes.Conn, attemptRes.Remote, nil)
		}
	}
	if err != nil {
		return nil, err
	}
	if m != nil && m.sessions != nil {
		m.sessions.Put(sess)
	}

	stream, err := sess.OpenStream(ctx, open)
	if err != nil {
		if m != nil && m.sessions != nil {
			m.sessions.Close(sess.Key(), dataplane.CloseReasonTransportFatal)
		} else {
			_ = sess.Close(dataplane.CloseReasonTransportFatal)
		}
		return nil, err
	}

	m.addFact(taskID, poc.Fact{TermID: "peer_id", Message: "peer_id=" + peerID})
	m.addFact(taskID, poc.Fact{TermID: "sid", Message: "sid=" + sid})
	dataProto := natHoleRespMsg.Protocol
	quicCC := natHoleRespMsg.QuicCC
	if len(attemptRes.TCPConns) > 0 {
		dataProto = string(dataplane.ProtocolTLS)
		quicCC = ""
	}

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
		PeerID:      peerID,
		AttemptPath: attemptRes.Path,
		AttemptWay:  attemptRes.Path,
		DataProto:   dataProto,
		PathFamily:  string(dpCfg.PathFamily),
		Portmap:     topologyPortmapEvidenceFromEvents(diagEvents.String()),
		StartedAt:   startedAt,
		Outcome:     "ok",
	})

	return &dialResult{
		stream:     stream,
		sid:        sid,
		dataProto:  dataProto,
		quicCC:     quicCC,
		attemptWay: attemptRes.Path,
	}, nil
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
