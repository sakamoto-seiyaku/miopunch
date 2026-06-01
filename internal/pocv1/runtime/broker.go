package runtime

import (
	"fmt"
	"net"
	"os"
	"strings"
	"time"

	"github.com/256dpi/gomqtt/broker"
	"github.com/256dpi/gomqtt/transport"
)

const defaultRuntimeBrokerEndpoint = "tcp://broker.emqx.io:1883"

type embeddedBroker struct {
	endpoint string
	server   transport.Server
	backend  *broker.MemoryBackend
	engine   *broker.Engine
}

func startEmbeddedBroker(endpoint string) (*embeddedBroker, error) {
	explicitEndpoint := strings.TrimSpace(endpoint) != ""
	url := normalizeBrokerEndpoint(endpoint)
	if !explicitEndpoint {
		url = "tcp://0.0.0.0:0"
	}
	server, err := transport.Launch(url)
	if err != nil {
		return nil, err
	}
	backend := broker.NewMemoryBackend()
	engine := broker.NewEngine(backend)
	engine.Accept(server)

	advertisedEndpoint := "tcp://" + server.Addr().String()
	if !explicitEndpoint {
		advertisedEndpoint, err = advertisedBrokerEndpoint(server.Addr(), brokerAdvertiseHost())
		if err != nil {
			_ = server.Close()
			backend.Close(500 * time.Millisecond)
			engine.Close()
			return nil, err
		}
	}
	return &embeddedBroker{
		endpoint: advertisedEndpoint,
		server:   server,
		backend:  backend,
		engine:   engine,
	}, nil
}

func (b *embeddedBroker) Endpoint() string {
	if b == nil {
		return ""
	}
	return b.endpoint
}

func (b *embeddedBroker) Close() error {
	if b == nil {
		return nil
	}
	if b.server != nil {
		_ = b.server.Close()
	}
	if b.backend != nil {
		b.backend.Close(500 * time.Millisecond)
	}
	if b.engine != nil {
		b.engine.Close()
	}
	return nil
}

func normalizeBrokerEndpoint(endpoint string) string {
	trimmed := strings.TrimSpace(endpoint)
	if trimmed == "" {
		return "tcp://127.0.0.1:0"
	}
	if strings.Contains(trimmed, "://") {
		return trimmed
	}
	return fmt.Sprintf("tcp://%s", trimmed)
}

func advertisedBrokerEndpoint(addr net.Addr, host string) (string, error) {
	if addr == nil {
		return "", fmt.Errorf("nil broker address")
	}
	_, port, err := net.SplitHostPort(addr.String())
	if err != nil {
		return "", fmt.Errorf("split broker address: %w", err)
	}
	host = strings.TrimSpace(host)
	if host == "" {
		return "", fmt.Errorf("empty broker advertise host")
	}
	return "tcp://" + net.JoinHostPort(host, port), nil
}

func brokerAdvertiseHost() string {
	if host := strings.TrimSpace(os.Getenv("MIOPUNCH_RUNTIME_BROKER_HOST")); host != "" {
		return host
	}

	addrs, err := net.InterfaceAddrs()
	if err == nil {
		for _, addr := range addrs {
			ipNet, ok := addr.(*net.IPNet)
			if !ok || ipNet == nil || ipNet.IP == nil {
				continue
			}
			ip := ipNet.IP.To4()
			if ip == nil || ip.IsLoopback() {
				continue
			}
			return ip.String()
		}
	}

	host, err := os.Hostname()
	if err == nil && strings.TrimSpace(host) != "" {
		return strings.TrimSpace(host)
	}
	return "127.0.0.1"
}
