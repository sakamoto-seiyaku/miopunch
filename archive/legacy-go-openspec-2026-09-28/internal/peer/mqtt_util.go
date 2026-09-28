package peer

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"net/url"
	"strings"

	"github.com/miopunch/miopunch/internal/netutil"
)

func mqttBrokerURLForLog(brokerURL string) string {
	u, err := url.Parse(brokerURL)
	if err != nil {
		return "invalid_mqtt_broker_url"
	}
	u.User = nil
	return u.String()
}

func buildMQTTBrokerURL(broker string, user string, pass string) (string, error) {
	broker = strings.TrimSpace(broker)
	if broker == "" {
		return "", errors.New("mqtt broker is required")
	}
	if !strings.Contains(broker, "://") {
		broker = "tcp://" + broker
	}
	u, err := url.Parse(broker)
	if err != nil {
		return "", fmt.Errorf("invalid mqtt broker url: %w", err)
	}
	if strings.TrimSpace(user) != "" {
		if pass != "" {
			u.User = url.UserPassword(user, pass)
		} else {
			u.User = url.User(user)
		}
	}
	return u.String(), nil
}

func resolveMQTTBrokerURL(ctx context.Context, brokerURL string, dnsMode string, dnsServers []string) (string, error) {
	u, err := url.Parse(brokerURL)
	if err != nil {
		return "", fmt.Errorf("invalid mqtt broker url: %w", err)
	}

	host := strings.TrimSpace(u.Hostname())
	if host == "" {
		return "", errors.New("mqtt broker host is required")
	}
	if _, err := netip.ParseAddr(host); err == nil {
		return brokerURL, nil
	}

	resolver, err := netutil.NewDNSResolver(dnsMode, dnsServers)
	if err != nil {
		return "", err
	}

	addrs, err := resolver.LookupNetIP(ctx, "ip", host)
	if err != nil {
		return "", err
	}
	if len(addrs) == 0 {
		return "", fmt.Errorf("mqtt broker hostname resolved to no addresses: %q", host)
	}

	port := u.Port()
	if port == "" {
		u.Host = addrs[0].String()
		return u.String(), nil
	}

	u.Host = net.JoinHostPort(addrs[0].String(), port)
	return u.String(), nil
}
