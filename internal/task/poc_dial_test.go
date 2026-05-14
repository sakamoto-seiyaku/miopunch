package task

import (
	"context"
	"io"
	"path/filepath"
	"testing"
	"time"

	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/pocstate"
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

type testPeerSession struct {
	key    dataplane.SessionKey
	closed bool
}

func (s *testPeerSession) Key() dataplane.SessionKey { return s.key }

func (s *testPeerSession) OpenStream(context.Context, dataplane.StreamOpen) (io.ReadWriteCloser, error) {
	return nil, io.ErrClosedPipe
}

func (s *testPeerSession) AcceptStream(context.Context) (*dataplane.AcceptedStream, error) {
	return nil, io.ErrClosedPipe
}

func (s *testPeerSession) Close(reason dataplane.CloseReason) error {
	s.closed = true
	return nil
}

func (s *testPeerSession) CloseReason() dataplane.CloseReason {
	if s.closed {
		return dataplane.CloseReasonDaemonShutdown
	}
	return ""
}

func (s *testPeerSession) Healthy() bool { return !s.closed }

func (s *testPeerSession) LastActivity() time.Time { return time.Unix(0, 0).UTC() }
