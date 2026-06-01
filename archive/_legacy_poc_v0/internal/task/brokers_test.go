package task

import (
	"testing"
)

func TestSelectReachableInviteBrokersPrefersCandidatesOutsideEffectivePair(t *testing.T) {
	brokerA := startTCPMQTTBroker(t)
	brokerB := startTCPMQTTBroker(t)
	brokerC := startTCPMQTTBroker(t)
	brokerD := startTCPMQTTBroker(t)

	m := NewManagerWithStatePath(t.TempDir() + "/state.json")
	t.Cleanup(m.Close)

	got, diagnostics, err := m.selectReachableInviteBrokers(
		[]string{brokerA, brokerB, brokerC, brokerD},
		[]string{brokerA, brokerB},
	)
	if err != nil {
		t.Fatalf("selectReachableInviteBrokers(...) error = %v, diagnostics=%v", err, diagnostics)
	}
	if len(got) != 2 {
		t.Fatalf("selectReachableInviteBrokers(...) = %v, want 2 brokers", got)
	}
	if got[0] != brokerC || got[1] != brokerD {
		t.Fatalf("selectReachableInviteBrokers(...) = %v, want [%q %q]", got, brokerC, brokerD)
	}
}

func TestSelectReachableInviteBrokersFallsBackToCurrentEffectivePair(t *testing.T) {
	brokerA := startTCPMQTTBroker(t)
	brokerB := startTCPMQTTBroker(t)
	brokerC := startTCPMQTTBroker(t)

	m := NewManagerWithStatePath(t.TempDir() + "/state.json")
	t.Cleanup(m.Close)

	got, diagnostics, err := m.selectReachableInviteBrokers(
		[]string{brokerA, brokerB, brokerC},
		[]string{brokerA, brokerB},
	)
	if err != nil {
		t.Fatalf("selectReachableInviteBrokers(...) error = %v, diagnostics=%v", err, diagnostics)
	}
	if len(got) != 2 {
		t.Fatalf("selectReachableInviteBrokers(...) = %v, want 2 brokers", got)
	}
	if got[0] != brokerC {
		t.Fatalf("selectReachableInviteBrokers(...)[0] = %q, want %q", got[0], brokerC)
	}
	if got[1] != brokerA && got[1] != brokerB {
		t.Fatalf("selectReachableInviteBrokers(...)[1] = %q, want one of %q or %q", got[1], brokerA, brokerB)
	}
}
