package dataplane

import (
	"fmt"
	"strings"
	"time"
)

type Protocol string

const (
	ProtocolKCP  Protocol = "kcp"
	ProtocolQUIC Protocol = "quic"
	ProtocolTLS  Protocol = "tls"
)

type QUICCC string

const (
	QUICCCBBR    QUICCC = "bbr"
	QUICCCBrutal QUICCC = "brutal"
)

type BrutalConfig struct {
	UpBps   uint64 // bytes/s
	DownBps uint64 // bytes/s (used for consistency checks; v1 doesn't auto-negotiate)
}

type Config struct {
	Proto  Protocol
	QuicCC QUICCC // only meaningful when Proto=quic

	Brutal BrutalConfig // only meaningful when QuicCC=brutal

	RemotePeerID string
	SecurityID   string
	SecretKey    []byte
	PathFamily   PathFamily
	IdleTimeout  time.Duration
}

func (c *Config) Normalize() {
	c.Proto = Protocol(strings.TrimSpace(string(c.Proto)))
	if c.Proto == "" {
		c.Proto = ProtocolQUIC
	}

	c.QuicCC = QUICCC(strings.TrimSpace(string(c.QuicCC)))
	if c.Proto == ProtocolQUIC && c.QuicCC == "" {
		c.QuicCC = QUICCCBBR
	}

	c.RemotePeerID = strings.TrimSpace(c.RemotePeerID)
	c.SecurityID = strings.TrimSpace(c.SecurityID)
	c.PathFamily = PathFamily(strings.TrimSpace(string(c.PathFamily)))
	if c.PathFamily == "" {
		c.PathFamily = PathFamilyUnknown
	}
}

func (c Config) Validate() error {
	c.Normalize()

	switch c.Proto {
	case ProtocolKCP:
		return nil
	case ProtocolQUIC:
		// ok
	case ProtocolTLS:
		return nil
	default:
		return fmt.Errorf("unsupported data proto: %q", c.Proto)
	}

	switch c.QuicCC {
	case QUICCCBBR:
		return nil
	case QUICCCBrutal:
		if c.Brutal.UpBps == 0 || c.Brutal.DownBps == 0 {
			return fmt.Errorf("brutal requires explicit up/down limits")
		}
		return nil
	default:
		return fmt.Errorf("unsupported quic cc: %q", c.QuicCC)
	}
}

func (c Config) sessionKey() SessionKey {
	c.Normalize()
	return SessionKey{
		RemotePeerID: c.RemotePeerID,
		Protocol:     c.Proto,
		SecurityID:   c.SecurityID,
		PathFamily:   c.PathFamily,
	}.Normalize()
}

func (c Config) requirePinnedIdentity() error {
	c.Normalize()
	if strings.TrimSpace(c.SecurityID) == "" {
		return fmt.Errorf("security_id is required")
	}
	if len(c.SecretKey) == 0 {
		return fmt.Errorf("secret_key is required")
	}
	return nil
}
