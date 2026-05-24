package controlplane

import (
	"context"
	"net/netip"
	"testing"
	"time"
)

type lookupFunc func(ctx context.Context, network string, host string) ([]netip.Addr, error)

func (f lookupFunc) LookupNetIP(ctx context.Context, network string, host string) ([]netip.Addr, error) {
	return f(ctx, network, host)
}

func TestJoinCodeValidate(t *testing.T) {
	c := JoinCode{
		InviteBrokers: []string{"192.0.2.1:1883"},
	}
	if err := c.Validate(); err != nil {
		t.Fatalf("JoinCode.Validate() error = %v", err)
	}
}

func TestJoinCodeValidate_RejectsCredentialsAndSchemes(t *testing.T) {
	tests := []struct {
		name   string
		broker string
	}{
		{name: "scheme", broker: "tcp://example.com:1883"},
		{name: "creds", broker: "user:pass@example.com:1883"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			c := JoinCode{InviteBrokers: []string{tt.broker}}
			if err := c.Validate(); err == nil {
				t.Fatalf("JoinCode.Validate() = nil, want error")
			}
		})
	}
}

func TestCanonicalizeInviteBrokers_ResolvesHostnameToIP(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	resolver := lookupFunc(func(_ context.Context, network string, host string) ([]netip.Addr, error) {
		if network != "ip4" || host != "broker.example.com" {
			return nil, nil
		}
		return []netip.Addr{netip.MustParseAddr("203.0.113.9")}, nil
	})

	got, warnings, err := CanonicalizeInviteBrokers(ctx, resolver, []string{"broker.example.com:1883"})
	if err != nil {
		t.Fatalf("CanonicalizeInviteBrokers() error = %v", err)
	}
	if len(warnings) != 0 {
		t.Fatalf("CanonicalizeInviteBrokers() warnings = %v, want none", warnings)
	}
	if len(got) != 1 || got[0] != "203.0.113.9:1883" {
		t.Fatalf("CanonicalizeInviteBrokers() = %v, want %v", got, []string{"203.0.113.9:1883"})
	}
}

func TestCanonicalizeInviteBrokers_UnresolvedHostnameKeepsHostnameAndWarns(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	resolver := lookupFunc(func(_ context.Context, network string, host string) ([]netip.Addr, error) {
		return nil, context.DeadlineExceeded
	})

	got, warnings, err := CanonicalizeInviteBrokers(ctx, resolver, []string{"broker.example.com:1883"})
	if err != nil {
		t.Fatalf("CanonicalizeInviteBrokers() error = %v", err)
	}
	if len(got) != 1 || got[0] != "broker.example.com:1883" {
		t.Fatalf("CanonicalizeInviteBrokers() = %v, want %v", got, []string{"broker.example.com:1883"})
	}
	if len(warnings) != 1 {
		t.Fatalf("CanonicalizeInviteBrokers() warnings = %v, want 1 warning", warnings)
	}
}

func TestSelectInviteBrokers_PrefersActive(t *testing.T) {
	got, err := SelectInviteBrokers([]string{"192.0.2.1:1883"}, []string{"198.51.100.1:1883"})
	if err != nil {
		t.Fatalf("SelectInviteBrokers() error = %v", err)
	}
	if len(got) != 1 || got[0] != "192.0.2.1:1883" {
		t.Fatalf("SelectInviteBrokers() = %v, want %v", got, []string{"192.0.2.1:1883"})
	}
}

func TestSelectInviteBrokers_FallsBackToEffective(t *testing.T) {
	got, err := SelectInviteBrokers(nil, []string{"198.51.100.1:1883", "198.51.100.2:1883"})
	if err != nil {
		t.Fatalf("SelectInviteBrokers() error = %v", err)
	}
	if len(got) != 2 || got[0] != "198.51.100.1:1883" || got[1] != "198.51.100.2:1883" {
		t.Fatalf("SelectInviteBrokers() = %v, want %v", got, []string{"198.51.100.1:1883", "198.51.100.2:1883"})
	}
}

func TestBrokerURLsForInviteJoin(t *testing.T) {
	got, err := BrokerURLsForInviteJoin([]string{"192.0.2.1:1883"})
	if err != nil {
		t.Fatalf("BrokerURLsForInviteJoin() error = %v", err)
	}
	if len(got) != 1 || got[0] != "tcp://192.0.2.1:1883" {
		t.Fatalf("BrokerURLsForInviteJoin() = %v, want %v", got, []string{"tcp://192.0.2.1:1883"})
	}
}

func TestJoinCodeBrokerURLsForInviteJoin(t *testing.T) {
	c := JoinCode{
		InviteBrokers: []string{"  192.0.2.1:1883  "},
	}

	got, err := c.BrokerURLsForInviteJoin()
	if err != nil {
		t.Fatalf("JoinCode.BrokerURLsForInviteJoin() error = %v", err)
	}
	if len(got) != 1 || got[0] != "tcp://192.0.2.1:1883" {
		t.Fatalf("JoinCode.BrokerURLsForInviteJoin() = %v, want %v", got, []string{"tcp://192.0.2.1:1883"})
	}
}
