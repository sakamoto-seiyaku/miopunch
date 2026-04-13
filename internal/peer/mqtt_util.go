package peer

import (
	"errors"
	"fmt"
	"net/url"
	"strings"
)

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
