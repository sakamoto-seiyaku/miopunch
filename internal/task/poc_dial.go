package task

import (
	"context"
	"errors"
	"io"
	"strings"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/punchdecision"
	"github.com/miopunch/miopunch/internal/punching"
	mqttsig "github.com/miopunch/miopunch/internal/signaling/mqtt"
	"github.com/miopunch/miopunch/internal/wire"
)

type dialResult struct {
	stream io.ReadWriteCloser

	sid        string
	dataProto  string
	quicCC     string
	attemptWay string
}

func (m *Manager) dialPeerStream(ctx context.Context, taskID string, peerID string, cfg pocstate.PeerConfig) (*dialResult, error) {
	if m != nil {
		m.mu.Lock()
		hook := m.dialPeerStreamHook
		m.mu.Unlock()
		if hook != nil {
			stream, err := hook(ctx, taskID, peerID, cfg)
			if err != nil {
				return nil, err
			}
			return &dialResult{stream: stream}, nil
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

	m.setStage(taskID, poc.StageCandidateExchange, "gather candidates")
	gather, err := connectivity.Gather(ctx, sid, connectivity.GatherConfig{
		ListenPort: 0,
		P2PNetwork: connectivity.P2PNetwork(cfg.P2PNetwork),
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
		TransactionID:  transactionID,
		ProxyName:      cfg.ProxyName,
		Protocol:       cfg.DataProto,
		QuicCC:         cfg.QUICCC,
		BrutalUpBps:    brutalUpBps,
		BrutalDownBps:  brutalDownBps,
		Capabilities:   []string{wire.CapabilityTCPP2PV0},
		P2PNetwork:     cfg.P2PNetwork,
		DirectAddrs:    gather.DirectAddrs,
		MappedAddrs:    gather.MappedAddrs,
		AssistedAddrs:  gather.AssistedAddrs,
		TCPDirectAddrs: gather.TCPDirectAddrs,
		TCPMappedAddrs: gather.TCPMappedAddrs,
		TCPSTUNCN:      gather.TCPSTUNCN,
		TCPSTUNGlobal:  gather.TCPSTUNGlobal,
		STUNCN:         gather.STUNCN,
		STUNGlobal:     gather.STUNGlobal,
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

	natHoleRespMsg, err := mq.RunVisitor(ctx, natHoleVisitorMsg, func(sid string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*wire.NatHoleResp, *wire.NatHoleResp, error) {
		return punchdecision.AnalyzeOnce(sid, visitor, client)
	})
	_ = mq.Close()
	if err != nil {
		return nil, err
	}

	m.setStage(taskID, poc.StagePunchAttempt, "punch attempt")
	attemptRes, err := connectivity.Attempt(ctx, sid, []byte(cfg.SecretKey), gather.UDP4Conn, gather.UDP6Conn, gather.TCP4Listener, gather.TCP6Listener, natHoleRespMsg, connectivity.AttemptConfig{
		P2PNetwork: connectivity.P2PNetwork(cfg.P2PNetwork),
	})
	if err != nil {
		return nil, err
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

	m.setStage(taskID, poc.StageDataplaneHandshake, "data plane dial stream")
	var stream io.ReadWriteCloser
	if len(attemptRes.TCPConns) > 0 {
		dpCfg.Proto = dataplane.ProtocolTLS
		stream, err = dataplane.DialTLSStream(ctx, sid, []byte(cfg.SecretKey), attemptRes.TCPConns, nil)
	} else {
		stream, err = dataplane.DialStream(ctx, dpCfg, attemptRes.Conn, attemptRes.Remote, nil)
	}
	if err != nil {
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
