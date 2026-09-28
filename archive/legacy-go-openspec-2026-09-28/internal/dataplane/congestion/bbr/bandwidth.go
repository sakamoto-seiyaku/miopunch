package bbr

import (
	"math"
	"time"

	"github.com/apernet/quic-go/congestion"
)

// NOTE: Derived from Hysteria2's BBR sender implementation.
// Source: apernet/hysteria (tag: app/v2.7.1), core/internal/congestion/bbr/bandwidth.go

const (
	infBandwidth = Bandwidth(math.MaxUint64)
)

// Bandwidth of a connection.
type Bandwidth uint64

const (
	// BitsPerSecond is 1 bit per second.
	BitsPerSecond Bandwidth = 1
	// BytesPerSecond is 1 byte per second.
	BytesPerSecond = 8 * BitsPerSecond
)

// BandwidthFromDelta calculates the bandwidth from a number of bytes and a time delta.
func BandwidthFromDelta(bytes congestion.ByteCount, delta time.Duration) Bandwidth {
	return Bandwidth(bytes) * Bandwidth(time.Second) / Bandwidth(delta) * BytesPerSecond
}
