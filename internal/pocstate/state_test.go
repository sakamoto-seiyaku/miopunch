package pocstate

import "testing"

func TestLocalConfigToPeerCarriesConnectivityOverrides(t *testing.T) {
	local := LocalConfig{
		ProxyName:            "p1",
		SecretKey:            "secret",
		MQTTBroker:           "broker:1883",
		TopicPrefix:          "miopunch/mnt01",
		V4Hint:               "direct",
		V6Hint:               "none",
		DataProto:            "quic",
		QUICCC:               "bbr",
		P2PNetwork:           "tcp_only",
		P2PIPFamily:          "v6",
		P2PPort:              5001,
		StunServers:          []string{"100.64.0.11:3478"},
		StunExplicit:         true,
		DisablePortMap:       true,
		DisableAssistedAddrs: true,
	}

	got := local.ToPeer()

	if got.P2PNetwork != local.P2PNetwork {
		t.Errorf("LocalConfig.ToPeer().P2PNetwork = %q, want %q", got.P2PNetwork, local.P2PNetwork)
	}
	if got.V4Hint != local.V4Hint {
		t.Errorf("LocalConfig.ToPeer().V4Hint = %q, want %q", got.V4Hint, local.V4Hint)
	}
	if got.V6Hint != local.V6Hint {
		t.Errorf("LocalConfig.ToPeer().V6Hint = %q, want %q", got.V6Hint, local.V6Hint)
	}
	if got.P2PIPFamily != local.P2PIPFamily {
		t.Errorf("LocalConfig.ToPeer().P2PIPFamily = %q, want %q", got.P2PIPFamily, local.P2PIPFamily)
	}
	if got.P2PPort != local.P2PPort {
		t.Errorf("LocalConfig.ToPeer().P2PPort = %d, want %d", got.P2PPort, local.P2PPort)
	}
	if len(got.StunServers) != 1 || got.StunServers[0] != local.StunServers[0] {
		t.Errorf("LocalConfig.ToPeer().StunServers = %v, want %v", got.StunServers, local.StunServers)
	}
	if !got.StunExplicit {
		t.Error("LocalConfig.ToPeer().StunExplicit = false, want true")
	}
	if !got.DisablePortMap {
		t.Error("LocalConfig.ToPeer().DisablePortMap = false, want true")
	}
	if !got.DisableAssistedAddrs {
		t.Error("LocalConfig.ToPeer().DisableAssistedAddrs = false, want true")
	}
}
