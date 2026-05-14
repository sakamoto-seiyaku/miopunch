package task

import (
	"context"
	"encoding/json"
	"io"
	"net"
	"path/filepath"
	"testing"
	"time"

	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/pocstate"
	"github.com/miopunch/miopunch/internal/shellproto"
)

func TestLoadPeerConfigUsesLocalDialDefaults(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	st := pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			MQTTBroker:           "100.65.0.10:1883",
			TopicPrefix:          "miopunch/test",
			DataProto:            "kcp",
			QUICCC:               "brutal",
			P2PNetwork:           "udp_only",
			P2PIPFamily:          "v4",
			P2PPort:              5000,
			StunServers:          []string{"100.65.0.11:3478"},
			StunExplicit:         true,
			DisablePortMap:       true,
			DisableAssistedAddrs: true,
		},
		Peers: map[string]pocstate.PeerConfig{
			"peer-a": {
				ProxyName:   "peer-a",
				SecretKey:   "secret",
				MQTTBroker:  "100.65.0.10:1883",
				TopicPrefix: "miopunch/test",
			},
		},
	}
	if err := pocstate.Save(statePath, st); err != nil {
		t.Fatalf("pocstate.Save() error = %v", err)
	}

	m := NewManagerWithStatePath(statePath)
	cfg, ok, err := m.loadPeerConfig("peer-a")
	if err != nil {
		t.Fatalf("loadPeerConfig(peer-a) error = %v", err)
	}
	if !ok {
		t.Fatal("loadPeerConfig(peer-a) ok = false, want true")
	}

	if cfg.P2PNetwork != "udp_only" {
		t.Errorf("loadPeerConfig(peer-a).P2PNetwork = %q, want %q", cfg.P2PNetwork, "udp_only")
	}
	if cfg.P2PIPFamily != "v4" {
		t.Errorf("loadPeerConfig(peer-a).P2PIPFamily = %q, want %q", cfg.P2PIPFamily, "v4")
	}
	if cfg.P2PPort != 0 {
		t.Errorf("loadPeerConfig(peer-a).P2PPort = %d, want %d", cfg.P2PPort, 0)
	}
	if len(cfg.StunServers) != 1 || cfg.StunServers[0] != "100.65.0.11:3478" {
		t.Errorf("loadPeerConfig(peer-a).StunServers = %v, want [100.65.0.11:3478]", cfg.StunServers)
	}
	if !cfg.StunExplicit {
		t.Error("loadPeerConfig(peer-a).StunExplicit = false, want true")
	}
	if !cfg.DisablePortMap {
		t.Error("loadPeerConfig(peer-a).DisablePortMap = false, want true")
	}
	if !cfg.DisableAssistedAddrs {
		t.Error("loadPeerConfig(peer-a).DisableAssistedAddrs = false, want true")
	}
}

func TestLoadPeerConfigKeepsPeerDialOverrides(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "state.json")
	st := pocstate.State{
		Format: pocstate.FormatV0,
		Local: &pocstate.LocalConfig{
			P2PNetwork:  "udp_only",
			P2PIPFamily: "v4",
			P2PPort:     5000,
			StunServers: []string{"100.65.0.11:3478"},
		},
		Peers: map[string]pocstate.PeerConfig{
			"peer-a": {
				ProxyName:    "peer-a",
				SecretKey:    "secret",
				MQTTBroker:   "100.65.0.10:1883",
				TopicPrefix:  "miopunch/test",
				P2PNetwork:   "tcp_only",
				P2PIPFamily:  "v6",
				P2PPort:      6000,
				StunServers:  []string{"100.65.0.99:3478"},
				StunExplicit: true,
			},
		},
	}
	if err := pocstate.Save(statePath, st); err != nil {
		t.Fatalf("pocstate.Save() error = %v", err)
	}

	m := NewManagerWithStatePath(statePath)
	cfg, ok, err := m.loadPeerConfig("peer-a")
	if err != nil {
		t.Fatalf("loadPeerConfig(peer-a) error = %v", err)
	}
	if !ok {
		t.Fatal("loadPeerConfig(peer-a) ok = false, want true")
	}

	if cfg.P2PNetwork != "tcp_only" {
		t.Errorf("loadPeerConfig(peer-a).P2PNetwork = %q, want %q", cfg.P2PNetwork, "tcp_only")
	}
	if cfg.P2PIPFamily != "v6" {
		t.Errorf("loadPeerConfig(peer-a).P2PIPFamily = %q, want %q", cfg.P2PIPFamily, "v6")
	}
	if cfg.P2PPort != 6000 {
		t.Errorf("loadPeerConfig(peer-a).P2PPort = %d, want %d", cfg.P2PPort, 6000)
	}
	if len(cfg.StunServers) != 1 || cfg.StunServers[0] != "100.65.0.99:3478" {
		t.Errorf("loadPeerConfig(peer-a).StunServers = %v, want [100.65.0.99:3478]", cfg.StunServers)
	}
}

func TestFindReusableSessionHonorsP2PNetwork(t *testing.T) {
	const (
		peerID = "peer-a"
		sid    = "sid-a"
	)

	udpSession := &testPeerSession{key: dataplane.SessionKey{
		RemotePeerID: peerID,
		Protocol:     dataplane.ProtocolQUIC,
		SecurityID:   sid,
		PathFamily:   dataplane.PathFamilyUDP4,
	}}
	tcpSession := &testPeerSession{key: dataplane.SessionKey{
		RemotePeerID: peerID,
		Protocol:     dataplane.ProtocolTLS,
		SecurityID:   sid,
		PathFamily:   dataplane.PathFamilyTCP4,
	}}

	tests := []struct {
		name      string
		cfg       pocstate.PeerConfig
		wantPath  dataplane.PathFamily
		wantProto dataplane.Protocol
		wantFound bool
		onlyUDP   bool
		onlyTCP   bool
	}{
		{
			name:      "tcp_only selects tcp session",
			cfg:       pocstate.PeerConfig{DataProto: "quic", P2PNetwork: "tcp_only"},
			wantPath:  dataplane.PathFamilyTCP4,
			wantProto: dataplane.ProtocolTLS,
			wantFound: true,
		},
		{
			name:      "udp_only selects udp session",
			cfg:       pocstate.PeerConfig{DataProto: "quic", P2PNetwork: "udp_only"},
			wantPath:  dataplane.PathFamilyUDP4,
			wantProto: dataplane.ProtocolQUIC,
			wantFound: true,
		},
		{
			name:      "auto can still reuse tls fallback",
			cfg:       pocstate.PeerConfig{DataProto: "kcp", P2PNetwork: "auto"},
			wantPath:  dataplane.PathFamilyTCP4,
			wantProto: dataplane.ProtocolTLS,
			wantFound: true,
			onlyTCP:   true,
		},
		{
			name:      "tcp_only rejects udp session",
			cfg:       pocstate.PeerConfig{DataProto: "quic", P2PNetwork: "tcp_only"},
			wantFound: false,
			onlyUDP:   true,
		},
		{
			name:      "udp_only rejects tcp session",
			cfg:       pocstate.PeerConfig{DataProto: "quic", P2PNetwork: "udp_only"},
			wantFound: false,
			onlyTCP:   true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			manager := dataplane.NewSessionManager()
			if !tt.onlyTCP {
				manager.Put(udpSession)
			}
			if !tt.onlyUDP {
				manager.Put(tcpSession)
			}

			got, ok := findReusableSession(manager, peerID, sid, tt.cfg)
			if ok != tt.wantFound {
				t.Fatalf("findReusableSession(%+v) ok = %v, want %v", tt.cfg, ok, tt.wantFound)
			}
			if !tt.wantFound {
				return
			}
			key := got.Key()
			if key.PathFamily != tt.wantPath {
				t.Errorf("findReusableSession(%+v).PathFamily = %q, want %q", tt.cfg, key.PathFamily, tt.wantPath)
			}
			if key.Protocol != tt.wantProto {
				t.Errorf("findReusableSession(%+v).Protocol = %q, want %q", tt.cfg, key.Protocol, tt.wantProto)
			}
		})
	}
}

func TestPassiveTopologyEvidenceRegistersSessionAndRuntime(t *testing.T) {
	const peerID = "peer-passive"
	startedAt := time.Unix(123, 456000000).UTC()
	sess := &testPeerSession{key: dataplane.SessionKey{
		Protocol:   dataplane.ProtocolQUIC,
		SecurityID: "sid-passive",
		PathFamily: dataplane.PathFamilyUDP4,
	}}

	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	defer m.Close()

	m.RegisterPassivePeerSession(peerID, sess)
	got, ok := m.sessions.Get(dataplane.SessionKey{
		RemotePeerID: peerID,
		Protocol:     dataplane.ProtocolQUIC,
		SecurityID:   "sid-passive",
		PathFamily:   dataplane.PathFamilyUDP4,
	})
	if !ok {
		t.Fatalf("RegisterPassivePeerSession(%q) session ok = false, want true", peerID)
	}
	if got.Key().RemotePeerID != peerID {
		t.Fatalf("RegisterPassivePeerSession(%q) key = %+v, want remote peer id", peerID, got.Key())
	}

	m.RecordPassiveTopologyAttempt(peerID, sess, "", startedAt, "ok")
	m.RecordPassiveTopologyPayload(peerID, "ping=ok")
	attempts, payloads := m.topologyRuntimeEvidence()
	if len(attempts) != 1 {
		t.Fatalf("topologyRuntimeEvidence() attempts length = %d, want 1: %#v", len(attempts), attempts)
	}
	if attempts[0].AttemptPath != "passive_accept_udp4" {
		t.Errorf("RecordPassiveTopologyAttempt().AttemptPath = %q, want %q", attempts[0].AttemptPath, "passive_accept_udp4")
	}
	if attempts[0].PeerID != peerID || attempts[0].DataProto != "quic" || attempts[0].PathFamily != "udp4" || attempts[0].Outcome != "ok" {
		t.Errorf("RecordPassiveTopologyAttempt() = %#v, want peer/proto/path/outcome evidence", attempts[0])
	}
	if len(payloads) != 1 || payloads[0].PeerID != peerID || payloads[0].Evidence != "ping=ok" {
		t.Fatalf("RecordPassiveTopologyPayload() payloads = %#v, want ping evidence for %q", payloads, peerID)
	}

	m.ClosePassivePeerSession(peerID, sess, dataplane.CloseReasonDaemonShutdown)
	if sess.CloseReason() != dataplane.CloseReasonDaemonShutdown {
		t.Fatalf("ClosePassivePeerSession(%q) close reason = %q, want %q", peerID, sess.CloseReason(), dataplane.CloseReasonDaemonShutdown)
	}
}

func TestDialedSessionIsActiveOnlyAfterApplicationSuccess(t *testing.T) {
	const peerID = "peer-active"
	key := dataplane.SessionKey{
		RemotePeerID: peerID,
		Protocol:     dataplane.ProtocolQUIC,
		SecurityID:   "sid-active",
		PathFamily:   dataplane.PathFamilyUDP4,
	}
	sess := &testPeerSession{key: key}
	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	defer m.Close()

	res := &dialResult{sess: sess}
	m.closeDialedSession(res, dataplane.CloseReasonStreamProtocolError)
	if !sess.closed {
		t.Fatalf("closeDialedSession(unmarked) closed = false, want true")
	}

	sess = &testPeerSession{key: key}
	res = &dialResult{sess: sess}
	m.markDialedSessionLive(res)
	if _, ok := m.sessions.Get(key); !ok {
		t.Fatalf("markDialedSessionLive() session ok = false, want true")
	}

	m.closeDialedSession(res, dataplane.CloseReasonStreamProtocolError)
	if _, ok := m.sessions.Get(key); ok {
		t.Fatalf("closeDialedSession(marked) session ok = true, want false")
	}
	if sess.CloseReason() != dataplane.CloseReasonStreamProtocolError {
		t.Fatalf("closeDialedSession(marked) close reason = %q, want %q", sess.CloseReason(), dataplane.CloseReasonStreamProtocolError)
	}
}

func TestKeepaliveActiveSessionsRecordsPayloadOnSuccess(t *testing.T) {
	const peerID = "peer-keepalive"
	sess := &testPeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: peerID,
			Protocol:     dataplane.ProtocolQUIC,
			SecurityID:   "sid-keepalive",
			PathFamily:   dataplane.PathFamilyUDP4,
		},
		openStream: newKeepaliveStream,
	}
	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	defer m.Close()
	m.sessions.Put(sess)

	m.keepaliveActiveSessions()

	if _, ok := m.sessions.Get(sess.key); !ok {
		t.Fatalf("keepaliveActiveSessions() session ok = false, want true")
	}
	_, payloads := m.topologyRuntimeEvidence()
	if len(payloads) != 1 || payloads[0].PeerID != peerID || payloads[0].Evidence != "keepalive=ok" {
		t.Fatalf("keepaliveActiveSessions() payloads = %#v, want keepalive evidence for %q", payloads, peerID)
	}
}

func TestKeepaliveActiveSessionsClosesFailedSession(t *testing.T) {
	const peerID = "peer-keepalive-fail"
	sess := &testPeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: peerID,
			Protocol:     dataplane.ProtocolQUIC,
			SecurityID:   "sid-keepalive-fail",
			PathFamily:   dataplane.PathFamilyUDP4,
		},
	}
	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	defer m.Close()
	m.sessions.Put(sess)

	m.keepaliveActiveSessions()

	if _, ok := m.sessions.Get(sess.key); ok {
		t.Fatalf("keepaliveActiveSessions() session ok = true, want false")
	}
	if sess.CloseReason() != dataplane.CloseReasonTransportFatal {
		t.Fatalf("keepaliveActiveSessions() close reason = %q, want %q", sess.CloseReason(), dataplane.CloseReasonTransportFatal)
	}
	attempts, _ := m.topologyRuntimeEvidence()
	if len(attempts) != 1 || attempts[0].PeerID != peerID || attempts[0].AttemptPath != "keepalive" || attempts[0].Outcome != "fail" {
		t.Fatalf("keepaliveActiveSessions() attempts = %#v, want keepalive failure for %q", attempts, peerID)
	}
}

func TestKeepaliveActiveSessionsSkipsPassiveSessions(t *testing.T) {
	const peerID = "peer-passive-keepalive"
	sess := &testPeerSession{
		key: dataplane.SessionKey{
			RemotePeerID: peerID,
			Protocol:     dataplane.ProtocolQUIC,
			SecurityID:   "sid-passive-keepalive",
			PathFamily:   dataplane.PathFamilyUDP4,
		},
	}
	m := NewManagerWithStatePath(filepath.Join(t.TempDir(), "state.json"))
	defer m.Close()
	m.sessions.Put(&passivePeerSession{PeerSession: sess, key: sess.key})

	m.keepaliveActiveSessions()

	if _, ok := m.sessions.Get(sess.key); !ok {
		t.Fatalf("keepaliveActiveSessions() passive session ok = false, want true")
	}
	attempts, payloads := m.topologyRuntimeEvidence()
	if len(attempts) != 0 || len(payloads) != 0 {
		t.Fatalf("keepaliveActiveSessions() evidence = (%#v, %#v), want none for passive session", attempts, payloads)
	}
}

func newKeepaliveStream(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
	client, server := net.Pipe()
	go func() {
		defer server.Close()
		_ = shellproto.WriteJSON(server, shellproto.Control{Op: shellproto.OpHello, OK: true})
		_, payload, err := shellproto.ReadFrame(server)
		if err != nil {
			return
		}
		var req shellproto.Control
		if err := json.Unmarshal(payload, &req); err != nil {
			return
		}
		if req.Op != shellproto.OpPing {
			return
		}
		_ = shellproto.WriteJSON(server, shellproto.Control{Op: shellproto.OpPing, OK: true})
	}()
	return client, nil
}

type testPeerSession struct {
	key         dataplane.SessionKey
	closed      bool
	closeReason dataplane.CloseReason
	openStream  func(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error)
}

func (s *testPeerSession) Key() dataplane.SessionKey { return s.key }

func (s *testPeerSession) OpenStream(ctx context.Context, open dataplane.StreamOpen) (io.ReadWriteCloser, error) {
	if s.openStream != nil {
		return s.openStream(ctx, open)
	}
	return nil, io.ErrClosedPipe
}

func (s *testPeerSession) AcceptStream(context.Context) (*dataplane.AcceptedStream, error) {
	return nil, io.ErrClosedPipe
}

func (s *testPeerSession) Close(reason dataplane.CloseReason) error {
	s.closed = true
	s.closeReason = reason
	return nil
}

func (s *testPeerSession) CloseReason() dataplane.CloseReason {
	return s.closeReason
}

func (s *testPeerSession) Healthy() bool { return !s.closed }

func (s *testPeerSession) LastActivity() time.Time { return time.Unix(0, 0).UTC() }
