package controlplane

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"strconv"
	"strings"
)

// JoinCode is the decoded join invitation payload (before wire encoding, e.g. bech32m).
//
// POC v0 constraint: join code MUST pin broker instance(s) for invite/join
// delivery via InviteBrokers.
type JoinCode struct {
	InviteBrokers []string `json:"invite_brokers"`
}

func (c JoinCode) Validate() error {
	return validateInviteBrokers(c.InviteBrokers)
}

// InviteBrokersForInviteJoin returns the broker endpoint list that MUST be used
// for invite/join MQTT delivery (subscribe/publish and reply paths).
func (c JoinCode) InviteBrokersForInviteJoin() ([]string, error) {
	if err := c.Validate(); err != nil {
		return nil, err
	}

	out := make([]string, 0, len(c.InviteBrokers))
	for _, ep := range c.InviteBrokers {
		out = append(out, strings.TrimSpace(ep))
	}
	return out, nil
}

// BrokerURLsForInviteJoin returns the MQTT connect URL list (tcp://host:port)
// that MUST be used for invite/join MQTT delivery (subscribe/publish and reply
// paths).
func (c JoinCode) BrokerURLsForInviteJoin() ([]string, error) {
	inviteBrokers, err := c.InviteBrokersForInviteJoin()
	if err != nil {
		return nil, err
	}
	return BrokerURLsForInviteJoin(inviteBrokers)
}

// HostnameResolver resolves hostnames into IP addresses.
//
// netutil.DNSResolver implements this interface.
type HostnameResolver interface {
	LookupNetIP(ctx context.Context, network string, host string) ([]netip.Addr, error)
}

// CanonicalizeInviteBrokers attempts to produce deterministic connectable
// addresses by resolving hostnames to IPs:
// - ip:port stays as-is (normalized with net.JoinHostPort)
// - hostname:port resolves the first A record and outputs ip:port
// - unresolved hostname keeps hostname:port, and returns a warning
//
// Do not use this helper for emitted invite-code brokers. Invite codes must
// preserve selected reachable hostname endpoints so joiners use the same broker
// endpoint that invite reachability probing validated.
func CanonicalizeInviteBrokers(ctx context.Context, resolver HostnameResolver, inviteBrokers []string) ([]string, []string, error) {
	if err := validateInviteBrokers(inviteBrokers); err != nil {
		return nil, nil, err
	}
	if resolver == nil {
		resolver = net.DefaultResolver
	}

	out := make([]string, 0, len(inviteBrokers))
	warnings := make([]string, 0, 1)
	seen := make(map[string]struct{}, len(inviteBrokers))
	for _, raw := range inviteBrokers {
		ep, err := parseBrokerEndpoint(raw)
		if err != nil {
			return nil, nil, err
		}

		canonical := net.JoinHostPort(ep.host, strconv.Itoa(int(ep.port)))
		if _, err := netip.ParseAddr(ep.host); err != nil {
			addrs, err := resolver.LookupNetIP(ctx, "ip4", ep.host)
			if err != nil || len(addrs) == 0 {
				warnings = append(warnings, fmt.Sprintf("invite_broker hostname not pinned: %s (dns/geo split may cause join failure; prefer ip:port or explicit control_plane.brokers)", canonical))
			} else {
				canonical = net.JoinHostPort(addrs[0].String(), strconv.Itoa(int(ep.port)))
			}
		}

		if _, ok := seen[canonical]; ok {
			continue
		}
		seen[canonical] = struct{}{}
		out = append(out, canonical)
	}
	return out, warnings, nil
}

// SelectInviteBrokers chooses up to two broker endpoints for join code output.
//
// Priority:
// 1) active brokers from up
// 2) brokers_effective fallback
func SelectInviteBrokers(activeBrokers, brokersEffective []string) ([]string, error) {
	src := activeBrokers
	if len(src) == 0 {
		src = brokersEffective
	}

	selected := make([]string, 0, 2)
	seen := make(map[string]struct{}, 2)
	for _, ep := range src {
		ep = strings.TrimSpace(ep)
		if ep == "" {
			continue
		}
		if _, ok := seen[ep]; ok {
			continue
		}
		seen[ep] = struct{}{}
		selected = append(selected, ep)
		if len(selected) == 2 {
			break
		}
	}

	if len(selected) == 0 {
		return nil, errors.New("no brokers available for invite_brokers")
	}
	if err := validateInviteBrokers(selected); err != nil {
		return nil, fmt.Errorf("select invite_brokers: %w", err)
	}
	return selected, nil
}

// BrokerURLsForInviteJoin converts join-code broker endpoints (host:port) into
// MQTT connect URLs (tcp://host:port).
func BrokerURLsForInviteJoin(inviteBrokers []string) ([]string, error) {
	if err := validateInviteBrokers(inviteBrokers); err != nil {
		return nil, err
	}

	urls := make([]string, 0, len(inviteBrokers))
	for _, ep := range inviteBrokers {
		ep = strings.TrimSpace(ep)
		urls = append(urls, "tcp://"+ep)
	}
	return urls, nil
}

func validateInviteBrokers(inviteBrokers []string) error {
	switch n := len(inviteBrokers); {
	case n == 0:
		return errors.New("invite_brokers is required")
	case n > 2:
		return errors.New("invite_brokers must contain at most 2 endpoints")
	}

	for _, ep := range inviteBrokers {
		if _, err := parseBrokerEndpoint(ep); err != nil {
			return err
		}
	}
	return nil
}

type brokerEndpoint struct {
	host string
	port uint16
}

func parseBrokerEndpoint(value string) (brokerEndpoint, error) {
	value = strings.TrimSpace(value)
	if value == "" {
		return brokerEndpoint{}, errors.New("empty broker endpoint")
	}
	if strings.Contains(value, "://") {
		return brokerEndpoint{}, fmt.Errorf("broker endpoint must be host:port, got %q", value)
	}
	if strings.Contains(value, "@") {
		return brokerEndpoint{}, fmt.Errorf("broker endpoint must not include credentials, got %q", value)
	}

	host, portStr, err := net.SplitHostPort(value)
	if err != nil {
		return brokerEndpoint{}, fmt.Errorf("invalid broker endpoint %q: %w", value, err)
	}
	host = strings.TrimSpace(host)
	if host == "" {
		return brokerEndpoint{}, fmt.Errorf("invalid broker endpoint %q: empty host", value)
	}

	portU64, err := strconv.ParseUint(portStr, 10, 16)
	if err != nil {
		return brokerEndpoint{}, fmt.Errorf("invalid broker endpoint %q: invalid port", value)
	}
	if portU64 == 0 {
		return brokerEndpoint{}, fmt.Errorf("invalid broker endpoint %q: invalid port", value)
	}

	return brokerEndpoint{
		host: host,
		port: uint16(portU64),
	}, nil
}
