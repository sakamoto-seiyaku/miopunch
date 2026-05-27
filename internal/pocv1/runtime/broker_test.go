package runtime

import (
	"net"
	"testing"
)

func TestAdvertisedBrokerEndpoint(t *testing.T) {
	t.Parallel()

	got, err := advertisedBrokerEndpoint(&net.TCPAddr{IP: net.IPv4zero, Port: 1883}, "10.0.0.5")
	if err != nil {
		t.Fatalf("advertisedBrokerEndpoint() error = %v, want nil", err)
	}
	if got != "tcp://10.0.0.5:1883" {
		t.Fatalf("advertisedBrokerEndpoint() = %q, want %q", got, "tcp://10.0.0.5:1883")
	}
}
