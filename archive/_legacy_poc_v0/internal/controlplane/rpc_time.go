package controlplane

import (
	"errors"
	"fmt"
	"time"
)

const (
	maxCreatedAtSkew = 10 * time.Minute
)

var (
	ErrClockSkew = errors.New("clock skew")

	ErrRPCRequestMissingExpiresAt = errors.New("rpc request expires_at_unix_ms is required")
	ErrRPCRequestExpired          = errors.New("rpc request expired")
)

// ValidateCreatedAtSkew validates the receiver-side clock-skew sanity rule:
// drop when abs(now-created_at) > 10 minutes.
//
// nowUnixMs and createdAtUnixMs are unix milliseconds (wall clock).
func ValidateCreatedAtSkew(nowUnixMs, createdAtUnixMs int64) error {
	deltaMs := nowUnixMs - createdAtUnixMs
	if deltaMs < 0 {
		deltaMs = -deltaMs
	}

	if time.Duration(deltaMs)*time.Millisecond <= maxCreatedAtSkew {
		return nil
	}

	return fmt.Errorf("%w: abs(now-created_at)=%s (check system time)", ErrClockSkew, time.Duration(deltaMs)*time.Millisecond)
}

// ValidateRPCRequestExpires validates the RPC request expiry rule:
// - expires_at_unix_ms is required
// - request is expired when now > expires_at
func ValidateRPCRequestExpires(nowUnixMs, expiresAtUnixMs int64) error {
	if expiresAtUnixMs <= 0 {
		return fmt.Errorf("%w", ErrRPCRequestMissingExpiresAt)
	}
	if nowUnixMs > expiresAtUnixMs {
		return fmt.Errorf("%w: now_unix_ms=%d expires_at_unix_ms=%d", ErrRPCRequestExpired, nowUnixMs, expiresAtUnixMs)
	}
	return nil
}

// ValidateRPCRequestTime applies the POC v0 receiver-side time validation rules.
//
//   - All messages are subject to the created_at clock-skew sanity drop.
//   - RPC requests (kind suffix "_request") additionally require expires_at_unix_ms
//     and are strictly expired when now > expires_at.
func ValidateRPCRequestTime(nowUnixMs int64, msg Message) error {
	if err := ValidateCreatedAtSkew(nowUnixMs, msg.Route.CreatedAtUnixMs); err != nil {
		return err
	}
	if IsRPCRequest(msg.Signed.Kind) {
		if err := ValidateRPCRequestExpires(nowUnixMs, msg.Route.ExpiresAtUnixMs); err != nil {
			return err
		}
	}
	return nil
}
