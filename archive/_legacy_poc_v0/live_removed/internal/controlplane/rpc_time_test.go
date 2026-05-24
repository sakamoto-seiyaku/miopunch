package controlplane

import (
	"errors"
	"testing"
	"time"
)

func TestValidateCreatedAtSkew_ExactThresholdAllowed(t *testing.T) {
	nowUnixMs := int64(maxCreatedAtSkew / time.Millisecond)
	createdAtUnixMs := int64(0)

	if err := ValidateCreatedAtSkew(nowUnixMs, createdAtUnixMs); err != nil {
		t.Fatalf("ValidateCreatedAtSkew() error = %v, want nil", err)
	}
}

func TestValidateCreatedAtSkew_OverThresholdDropped(t *testing.T) {
	nowUnixMs := int64(maxCreatedAtSkew/time.Millisecond) + 1
	createdAtUnixMs := int64(0)

	err := ValidateCreatedAtSkew(nowUnixMs, createdAtUnixMs)
	if !errors.Is(err, ErrClockSkew) {
		t.Fatalf("ValidateCreatedAtSkew() error = %v, want ErrClockSkew", err)
	}
}

func TestValidateRPCRequestExpires_Missing(t *testing.T) {
	err := ValidateRPCRequestExpires(100, 0)
	if !errors.Is(err, ErrRPCRequestMissingExpiresAt) {
		t.Fatalf("ValidateRPCRequestExpires() error = %v, want ErrRPCRequestMissingExpiresAt", err)
	}
}

func TestValidateRPCRequestExpires_Expired(t *testing.T) {
	err := ValidateRPCRequestExpires(200, 199)
	if !errors.Is(err, ErrRPCRequestExpired) {
		t.Fatalf("ValidateRPCRequestExpires() error = %v, want ErrRPCRequestExpired", err)
	}
}

func TestValidateRPCRequestTime_NonRequestDoesNotRequireExpires(t *testing.T) {
	msg := Message{
		Route: Route{
			CreatedAtUnixMs: 0,
		},
		Signed: Signed{
			Kind: "best_effort",
		},
	}

	if err := ValidateRPCRequestTime(0, msg); err != nil {
		t.Fatalf("ValidateRPCRequestTime() error = %v, want nil", err)
	}
}

func TestValidateRPCRequestTime_RequestRequiresExpires(t *testing.T) {
	msg := Message{
		Route: Route{
			CreatedAtUnixMs: 0,
			ExpiresAtUnixMs: 0,
		},
		Signed: Signed{
			Kind: "echo_request",
		},
	}

	err := ValidateRPCRequestTime(0, msg)
	if !errors.Is(err, ErrRPCRequestMissingExpiresAt) {
		t.Fatalf("ValidateRPCRequestTime() error = %v, want ErrRPCRequestMissingExpiresAt", err)
	}
}
