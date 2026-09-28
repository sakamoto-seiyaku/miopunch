package mqtt

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"io"
	"net"
	"os"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/miopunch/miopunch/internal/pocv1/peere2e"
)

// Keep the default aligned with the legacy built-in public broker list, but pin
// the current remote test to the candidate that actually completes a full MQTT
// roundtrip in this environment.
const defaultRemotePeerMessageBrokerURL = "tcp://broker.emqx.io:1883"

func TestPeerMessageSession_RemoteBrokerRoundTrip(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()

	brokerURL := remotePeerMessageBrokerURL()
	topic := "miopunch/test/v1/peer/remote/" + randomTopicSuffix(t)

	receiver, err := OpenPeerMessageSession(ctx, PeerMessageConfig{
		BrokerURL:       brokerURL,
		SubscribeTopics: []string{topic},
	})
	if err != nil {
		if isRemoteMQTTAvailabilityError(err) {
			t.Skipf("remote mqtt receiver unavailable (%s): %v", brokerURL, err)
		}
		t.Fatalf("OpenPeerMessageSession(receiver, %s) error = %v, want nil", brokerURL, err)
	}
	defer receiver.Close()

	sender, err := OpenPeerMessageSession(ctx, PeerMessageConfig{
		BrokerURL: brokerURL,
	})
	if err != nil {
		if isRemoteMQTTAvailabilityError(err) {
			t.Skipf("remote mqtt sender unavailable (%s): %v", brokerURL, err)
		}
		t.Fatalf("OpenPeerMessageSession(sender, %s) error = %v, want nil", brokerURL, err)
	}
	defer sender.Close()

	fx := mustPeerMessageFixture(t)
	sealedOuter, err := sender.PublishInner(ctx, topic, fx.inner, fx.recipientX25519Pub, peere2e.SealOptions{
		EphemeralPrivateKey: fx.ephemeralX25519Priv,
		Nonce:               fx.nonce,
	})
	if err != nil {
		if isRemoteMQTTAvailabilityError(err) {
			t.Skipf("remote mqtt publish unavailable (%s): %v", brokerURL, err)
		}
		t.Fatalf("PublishInner(%s) error = %v, want nil", brokerURL, err)
	}

	opened, err := receiver.WaitOpened(ctx, fx.recipientX25519Priv, peere2e.OpenOptions{})
	if err != nil {
		if isRemoteMQTTAvailabilityError(err) {
			t.Skipf("remote mqtt receive unavailable (%s): %v", brokerURL, err)
		}
		t.Fatalf("WaitOpened(%s) error = %v, want nil", brokerURL, err)
	}
	if opened.Topic != topic {
		t.Fatalf("WaitOpened(%s).Topic = %q, want %q", brokerURL, opened.Topic, topic)
	}
	if diff := diffInner(fx.inner, opened.Inner); diff != "" {
		t.Fatalf("WaitOpened(%s) inner mismatch (-want +got):\n%s", brokerURL, diff)
	}

	wantPayload, err := sealedOuter.MarshalBinary()
	if err != nil {
		t.Fatalf("sealedOuter.MarshalBinary(%s) error = %v, want nil", brokerURL, err)
	}
	gotPayload, err := opened.Outer.MarshalBinary()
	if err != nil {
		t.Fatalf("opened.Outer.MarshalBinary(%s) error = %v, want nil", brokerURL, err)
	}
	if !bytes.Equal(gotPayload, wantPayload) {
		t.Fatalf("opened.Outer.MarshalBinary(%s) = %x, want %x", brokerURL, gotPayload, wantPayload)
	}
}

func remotePeerMessageBrokerURL() string {
	if value := strings.TrimSpace(os.Getenv("MIOPUNCH_REMOTE_MQTT_BROKER_URL")); value != "" {
		if !strings.Contains(value, "://") {
			return "tcp://" + value
		}
		return value
	}
	return defaultRemotePeerMessageBrokerURL
}

func randomTopicSuffix(t *testing.T) string {
	t.Helper()

	var buf [8]byte
	if _, err := rand.Read(buf[:]); err != nil {
		t.Fatalf("rand.Read(topic suffix) error = %v, want nil", err)
	}
	return hex.EncodeToString(buf[:])
}

func isRemoteMQTTAvailabilityError(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) || errors.Is(err, io.EOF) {
		return true
	}

	var netErr net.Error
	if errors.As(err, &netErr) {
		return true
	}
	var opErr *net.OpError
	if errors.As(err, &opErr) {
		return true
	}
	var dnsErr *net.DNSError
	if errors.As(err, &dnsErr) {
		return true
	}

	message := strings.ToLower(err.Error())
	return strings.Contains(message, "future canceled") ||
		strings.Contains(message, "connection refused") ||
		strings.Contains(message, "connection reset") ||
		strings.Contains(message, "connection timed out") ||
		strings.Contains(message, "context deadline exceeded") ||
		strings.Contains(message, "dial tcp") ||
		strings.Contains(message, "eof") ||
		strings.Contains(message, "host is down") ||
		strings.Contains(message, "i/o timeout") ||
		strings.Contains(message, "network is unreachable") ||
		strings.Contains(message, "no route to host") ||
		strings.Contains(message, "server unavailable") ||
		strings.Contains(message, syscall.ECONNREFUSED.Error())
}
