package stunclient

import (
	"errors"
	"fmt"
	"strings"
)

type EndpointScheme string

const (
	EndpointSchemeDual EndpointScheme = "dual"
	EndpointSchemeUDP  EndpointScheme = "udp"
	EndpointSchemeTCP  EndpointScheme = "tcp"
)

var (
	ErrEmptyEndpoint             = errors.New("empty stun endpoint")
	ErrUnsupportedEndpointScheme = errors.New("unsupported stun scheme")
	ErrUnsupportedEndpointFormat = errors.New("unsupported stun endpoint format")
)

type ParsedEndpoint struct {
	Raw      string
	HostPort string
	Scheme   EndpointScheme
}

func ParseEndpoint(raw string) (ParsedEndpoint, error) {
	trimmed := strings.TrimSpace(raw)
	if trimmed == "" {
		return ParsedEndpoint{}, ErrEmptyEndpoint
	}
	if strings.Contains(trimmed, "?") {
		return ParsedEndpoint{}, ErrUnsupportedEndpointFormat
	}

	scheme := EndpointSchemeDual
	hostPort := trimmed
	switch {
	case strings.HasPrefix(trimmed, "udp://"):
		scheme = EndpointSchemeUDP
		hostPort = strings.TrimPrefix(trimmed, "udp://")
	case strings.HasPrefix(trimmed, "tcp://"):
		scheme = EndpointSchemeTCP
		hostPort = strings.TrimPrefix(trimmed, "tcp://")
	case strings.Contains(trimmed, "://"):
		return ParsedEndpoint{}, ErrUnsupportedEndpointScheme
	}

	hostPort = strings.TrimSpace(hostPort)
	if hostPort == "" {
		return ParsedEndpoint{}, fmt.Errorf("%w: missing host:port", ErrUnsupportedEndpointFormat)
	}

	return ParsedEndpoint{
		Raw:      trimmed,
		HostPort: hostPort,
		Scheme:   scheme,
	}, nil
}

// FilterHostPorts filters STUN endpoints for the requested network.
//
// Scheme semantics:
// - dual (host:port) is usable for both UDP and TCP.
// - udp:// is usable only for UDP.
// - tcp:// is usable only for TCP.
//
// Parse errors are returned in the errors slice and the corresponding entry is
// dropped.
func FilterHostPorts(raw []string, want EndpointScheme) (usable []string, ignored []string, errors []string) {
	usable = make([]string, 0, len(raw))
	ignored = make([]string, 0)
	errors = make([]string, 0)

	want = EndpointScheme(strings.TrimSpace(strings.ToLower(string(want))))
	if want != EndpointSchemeUDP && want != EndpointSchemeTCP {
		errors = append(errors, fmt.Sprintf("invalid want scheme: %q", want))
		return usable, ignored, errors
	}

	for _, entry := range raw {
		parsed, err := ParseEndpoint(entry)
		if err != nil {
			errors = append(errors, fmt.Sprintf("%s: %v", strings.TrimSpace(entry), err))
			continue
		}

		if (want == EndpointSchemeUDP && parsed.Scheme == EndpointSchemeTCP) ||
			(want == EndpointSchemeTCP && parsed.Scheme == EndpointSchemeUDP) {
			ignored = append(ignored, parsed.Raw)
			continue
		}
		usable = append(usable, parsed.HostPort)
	}

	return usable, ignored, errors
}
