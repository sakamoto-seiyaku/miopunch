package http_panel

import (
	"fmt"
	"net/http"
	"net/url"
	"strings"
)

type originError struct {
	OriginHeader  string
	RefererHeader string
	Allowed       []string
	Message       string
}

func (e *originError) Error() string {
	return e.Message
}

func (s *Server) requireSameOrigin(r *http.Request) *originError {
	allowed := s.allowedOriginList()

	originHeader := strings.TrimSpace(r.Header.Get("Origin"))
	if originHeader != "" {
		origin, err := canonicalOrigin(originHeader)
		if err != nil {
			return &originError{
				OriginHeader:  originHeader,
				RefererHeader: strings.TrimSpace(r.Header.Get("Referer")),
				Allowed:       allowed,
				Message:       "invalid Origin header",
			}
		}

		if s.isAllowedOrigin(origin) {
			return nil
		}

		return &originError{
			OriginHeader:  originHeader,
			RefererHeader: strings.TrimSpace(r.Header.Get("Referer")),
			Allowed:       allowed,
			Message:       "Origin does not match panel origin",
		}
	}

	refererHeader := strings.TrimSpace(r.Header.Get("Referer"))
	if refererHeader != "" {
		origin, err := canonicalOrigin(refererHeader)
		if err != nil {
			return &originError{
				OriginHeader:  originHeader,
				RefererHeader: refererHeader,
				Allowed:       allowed,
				Message:       "invalid Referer header",
			}
		}

		if s.isAllowedOrigin(origin) {
			return nil
		}

		return &originError{
			OriginHeader:  originHeader,
			RefererHeader: refererHeader,
			Allowed:       allowed,
			Message:       "Referer does not match panel origin",
		}
	}

	return &originError{
		OriginHeader:  originHeader,
		RefererHeader: refererHeader,
		Allowed:       allowed,
		Message:       "missing Origin/Referer header",
	}
}

func canonicalOrigin(raw string) (string, error) {
	u, err := url.Parse(strings.TrimSpace(raw))
	if err != nil {
		return "", err
	}
	if strings.TrimSpace(u.Scheme) == "" || strings.TrimSpace(u.Host) == "" {
		return "", fmt.Errorf("missing scheme/host")
	}
	return u.Scheme + "://" + u.Host, nil
}

func (s *Server) isAllowedOrigin(origin string) bool {
	u, err := url.Parse(origin)
	if err != nil {
		return false
	}
	if strings.TrimSpace(u.Scheme) != "http" {
		return false
	}
	if strings.TrimSpace(u.Host) == "" {
		return false
	}
	_, ok := s.allowedHosts[u.Host]
	return ok
}

func (s *Server) allowedOriginList() []string {
	out := make([]string, 0, len(s.allowedHosts))
	for host := range s.allowedHosts {
		out = append(out, "http://"+host)
	}
	return out
}
