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

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/punchdecision"
	"github.com/miopunch/miopunch/internal/punching"
	mqttsig "github.com/miopunch/miopunch/internal/signaling/mqtt"
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
			return &dialResult{stream: stream, legacyHello: true}, nil
		}
	}

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
		reuseKey := dataplane.SessionKey{
			RemotePeerID: peerID,
			Protocol:     dataplane.Protocol(cfg.DataProto),
			SecurityID:   sid,
		}
		sess, ok := m.sessions.Find(reuseKey)
		if !ok {
			reuseKey.Protocol = dataplane.ProtocolTLS
			sess, ok = m.sessions.Find(reuseKey)
		}
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

	m.setStage(taskID, poc.StageCandidateExchange, "gather candidates")
	gather, err := connectivity.Gather(ctx, sid, connectivity.GatherConfig{
		ListenPort:           cfg.P2PPort,
		P2PNetwork:           connectivity.P2PNetwork(cfg.P2PNetwork),
		P2PIPFamily:          connectivity.P2PIPFamily(cfg.P2PIPFamily),
		DisableAssistedAddrs: cfg.DisableAssistedAddrs,
		DisablePortMap:       cfg.DisablePortMap,
		StunServers:          cfg.StunServers,
		StunExplicit:         cfg.StunExplicit,
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
	mq, err := mqttsig.Open(ctx, mqttsig.Config{
		BrokerURL:       mqttBrokerURL(cfg.MQTTBroker),
		TopicPrefix:     cfg.TopicPrefix,
		SID:             sid,
		Role:            mqttsig.RoleVisitor,
		HelloTimeout:    10 * time.Second,
		ExchangeTimeout: 10 * time.Second,
		BarrierTimeout:  10 * time.Second,
	})
	if err != nil {
		return nil, err
	}

	var decisionRes *punchdecision.Result
	natHoleRespMsg, err := mq.RunVisitor(ctx, natHoleVisitorMsg, func(sid string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*wire.NatHoleResp, *wire.NatHoleResp, error) {
		res, err := punchdecision.AnalyzeWithDaemonMemory(sid, peerID, visitor, client)
		if err != nil {
			return nil, nil, err
		}
		decisionRes = res
		return res.VisitorResponse, res.ClientResponse, nil
	})
	_ = mq.Close()
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

	var attemptEvents bytes.Buffer
	attemptEmitter := event.NewEmitter(&attemptEvents, "task")

	m.setStage(taskID, poc.StagePunchAttempt, "punch attempt")
	attemptRes, err := connectivity.Attempt(ctx, sid, []byte(cfg.SecretKey), gather.UDP4Conn, gather.UDP6Conn, gather.TCP4Listener, gather.TCP6Listener, natHoleRespMsg, connectivity.AttemptConfig{
		P2PNetwork:  connectivity.P2PNetwork(cfg.P2PNetwork),
		P2PIPFamily: connectivity.P2PIPFamily(cfg.P2PIPFamily),
		Emitter:     attemptEmitter,
	})
	if err != nil {
		for _, line := range strings.Split(attemptEvents.String(), "\n") {
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
	if attemptRes.Conn == gather.UDP4Conn {
		gather.UDP4Conn = nil
	}
	if attemptRes.Conn == gather.UDP6Conn {
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
		sess, err = dataplane.DialSession(ctx, dpCfg, attemptRes.Conn, attemptRes.Remote, nil)
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
	cfg.NormalizeDefaults()
	return cfg, true, nil
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
