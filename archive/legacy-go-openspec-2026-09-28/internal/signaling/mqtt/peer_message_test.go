package mqtt

import (
	"bytes"
	"context"
	"crypto/ecdh"
	"crypto/ed25519"
	"errors"
	"io"
	"testing"
	"time"

	"github.com/256dpi/gomqtt/broker"
	"github.com/256dpi/gomqtt/packet"
	"github.com/256dpi/gomqtt/transport"

	"github.com/miopunch/miopunch/internal/pocv1/peere2e"
	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
)

func TestPeerMessageSession_RoundTripUsesBinaryOuterPayload(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	brokerURL, cleanup := launchPeerMessageBroker(t)
	defer cleanup()

	topic := "miopunch/test/v1/peer/demo"
	sender, receiver := mustPeerMessageSessions(t, ctx, brokerURL, topic)
	defer sender.Close()
	defer receiver.Close()

	fx := mustPeerMessageFixture(t)
	sealedOuter, err := sender.PublishInner(ctx, topic, fx.inner, fx.recipientX25519Pub, peere2e.SealOptions{
		EphemeralPrivateKey: fx.ephemeralX25519Priv,
		Nonce:               fx.nonce,
	})
	if err != nil {
		t.Fatalf("PublishInner() error = %v, want nil", err)
	}

	got, err := receiver.WaitOpened(ctx, fx.recipientX25519Priv, peere2e.OpenOptions{})
	if err != nil {
		t.Fatalf("WaitOpened() error = %v, want nil", err)
	}
	if got.Topic != topic {
		t.Fatalf("WaitOpened().Topic = %q, want %q", got.Topic, topic)
	}
	if diff := diffInner(fx.inner, got.Inner); diff != "" {
		t.Fatalf("WaitOpened() inner mismatch (-want +got):\n%s", diff)
	}
	if got.Inner.SenderPeerID != fx.inner.SenderPeerID {
		t.Fatalf("WaitOpened().Inner.SenderPeerID = %q, want %q", got.Inner.SenderPeerID, fx.inner.SenderPeerID)
	}

	wantPayload, err := sealedOuter.MarshalBinary()
	if err != nil {
		t.Fatalf("sealedOuter.MarshalBinary() error = %v, want nil", err)
	}
	gotPayload, err := got.Outer.MarshalBinary()
	if err != nil {
		t.Fatalf("got.Outer.MarshalBinary() error = %v, want nil", err)
	}
	if !bytes.Equal(gotPayload, wantPayload) {
		t.Fatalf("WaitOpened().Outer.MarshalBinary() = %x, want %x", gotPayload, wantPayload)
	}
}

func TestPeerMessageSession_WaitOpenedDropsMalformedAndKeepsWaiting(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	brokerURL, cleanup := launchPeerMessageBroker(t)
	defer cleanup()

	topic := "miopunch/test/v1/peer/drop"
	sender, receiver := mustPeerMessageSessions(t, ctx, brokerURL, topic)
	defer sender.Close()
	defer receiver.Close()

	fx := mustPeerMessageFixture(t)
	waitErrCh := make(chan error, 1)
	openedCh := make(chan OpenedPeerMessage, 1)
	go func() {
		opened, err := receiver.WaitOpened(ctx, fx.recipientX25519Priv, peere2e.OpenOptions{})
		if err != nil {
			waitErrCh <- err
			return
		}
		openedCh <- opened
	}()

	if err := sender.publishPayload(ctx, topic, []byte{0x01, 0x02, 0x03}); err != nil {
		t.Fatalf("publishPayload(malformed) error = %v, want nil", err)
	}

	select {
	case err := <-waitErrCh:
		t.Fatalf("WaitOpened() returned after malformed payload: %v", err)
	case opened := <-openedCh:
		t.Fatalf("WaitOpened() accepted malformed payload: %#v", opened)
	case <-time.After(200 * time.Millisecond):
	}

	if _, err := sender.PublishInner(ctx, topic, fx.inner, fx.recipientX25519Pub, peere2e.SealOptions{
		EphemeralPrivateKey: fx.ephemeralX25519Priv,
		Nonce:               fx.nonce,
	}); err != nil {
		t.Fatalf("PublishInner(valid) error = %v, want nil", err)
	}

	select {
	case err := <-waitErrCh:
		t.Fatalf("WaitOpened() error = %v, want nil", err)
	case opened := <-openedCh:
		if diff := diffInner(fx.inner, opened.Inner); diff != "" {
			t.Fatalf("WaitOpened() inner mismatch after malformed payload (-want +got):\n%s", diff)
		}
	case <-ctx.Done():
		t.Fatalf("timed out waiting for valid payload: %v", ctx.Err())
	}
}

func launchPeerMessageBroker(t *testing.T) (string, func()) {
	t.Helper()

	server, err := transport.Launch("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("transport.Launch() error = %v, want nil", err)
	}

	backend := broker.NewMemoryBackend()
	engine := broker.NewEngine(backend)
	engine.Accept(server)

	cleanup := func() {
		_ = server.Close()
		engine.Close()
	}
	return "tcp://" + server.Addr().String(), cleanup
}

func mustPeerMessageSessions(t *testing.T, ctx context.Context, brokerURL string, topic string) (*PeerMessageSession, *PeerMessageSession) {
	t.Helper()

	sender, err := OpenPeerMessageSession(ctx, PeerMessageConfig{
		BrokerURL:       brokerURL,
		SubscribeTopics: []string{topic},
	})
	if err != nil {
		t.Fatalf("OpenPeerMessageSession(sender) error = %v, want nil", err)
	}
	receiver, err := OpenPeerMessageSession(ctx, PeerMessageConfig{
		BrokerURL:       brokerURL,
		SubscribeTopics: []string{topic},
	})
	if err != nil {
		sender.Close()
		t.Fatalf("OpenPeerMessageSession(receiver) error = %v, want nil", err)
	}
	return sender, receiver
}

type peerMessageFixture struct {
	inner               pocwire.InnerMessage
	recipientX25519Priv []byte
	recipientX25519Pub  []byte
	ephemeralX25519Priv []byte
	nonce               []byte
}

func mustPeerMessageFixture(t *testing.T) peerMessageFixture {
	t.Helper()

	senderSeed := bytes.Repeat([]byte{0x11}, ed25519.SeedSize)
	senderPriv := ed25519.NewKeyFromSeed(senderSeed)
	senderPub := senderPriv.Public().(ed25519.PublicKey)
	senderPeerID, err := pocwire.PeerIDFromEd25519Pub(senderPub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(sender) error = %v, want nil", err)
	}

	recipientSeed := bytes.Repeat([]byte{0x22}, ed25519.SeedSize)
	recipientPriv := ed25519.NewKeyFromSeed(recipientSeed)
	recipientPub := recipientPriv.Public().(ed25519.PublicKey)
	recipientPeerID, err := pocwire.PeerIDFromEd25519Pub(recipientPub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(recipient) error = %v, want nil", err)
	}

	msgID, err := pocwire.CanonicalizeMsgID("JBSWY3DPEHPK3PXPJBSWY3DPAA")
	if err != nil {
		t.Fatalf("CanonicalizeMsgID() error = %v, want nil", err)
	}

	inner := pocwire.InnerMessage{
		DstPeerID:       recipientPeerID,
		MsgID:           msgID,
		CreatedAtUnixMs: 1_717_000_000_000,
		ExpiresAtUnixMs: 1_717_000_030_000,
		SenderPeerID:    senderPeerID,
		SenderEd25519:   append([]byte(nil), senderPub...),
		Kind:            pocwire.KindJoinRequest,
		Body:            []byte(`{"invite_id":"INV-01","reply_topic":"mp/v1/reply/demo"}`),
	}
	if err := pocwire.SignInner(senderPriv, &inner); err != nil {
		t.Fatalf("SignInner() error = %v, want nil", err)
	}

	recipientX25519Priv := bytes.Repeat([]byte{0x33}, 32)
	recipientX25519Pub, err := x25519Public(recipientX25519Priv)
	if err != nil {
		t.Fatalf("x25519Public(recipient) error = %v, want nil", err)
	}

	return peerMessageFixture{
		inner:               inner,
		recipientX25519Priv: recipientX25519Priv,
		recipientX25519Pub:  recipientX25519Pub,
		ephemeralX25519Priv: bytes.Repeat([]byte{0x44}, 32),
		nonce:               bytes.Repeat([]byte{0x55}, 24),
	}
}

func diffInner(want, got pocwire.InnerMessage) string {
	switch {
	case want.DstPeerID != got.DstPeerID:
		return "dst_peer_id mismatch"
	case want.MsgID != got.MsgID:
		return "msg_id mismatch"
	case want.CreatedAtUnixMs != got.CreatedAtUnixMs:
		return "created_at_unix_ms mismatch"
	case want.ExpiresAtUnixMs != got.ExpiresAtUnixMs:
		return "expires_at_unix_ms mismatch"
	case want.SenderPeerID != got.SenderPeerID:
		return "sender_peer_id mismatch"
	case !bytes.Equal(want.SenderEd25519, got.SenderEd25519):
		return "sender_ed25519 mismatch"
	case want.Kind != got.Kind:
		return "kind mismatch"
	case want.InReplyTo != got.InReplyTo:
		return "in_reply_to mismatch"
	case !bytes.Equal(want.Body, got.Body):
		return "body mismatch"
	case !bytes.Equal(want.Signature, got.Signature):
		return "signature mismatch"
	default:
		return ""
	}
}

func x25519Public(rawPrivate []byte) ([]byte, error) {
	priv, err := ecdh.X25519().NewPrivateKey(rawPrivate)
	if err != nil {
		return nil, err
	}
	return priv.PublicKey().Bytes(), nil
}

func TestPeerMessageSession_OpenPeerMessageSessionRejectsBlankTopic(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	brokerURL, cleanup := launchPeerMessageBroker(t)
	defer cleanup()

	_, err := OpenPeerMessageSession(ctx, PeerMessageConfig{
		BrokerURL:       brokerURL,
		SubscribeTopics: []string{" "},
	})
	if err == nil {
		t.Fatalf("OpenPeerMessageSession(blank topic) error = nil, want non-nil")
	}
}

func TestPeerMessageSession_PublishInnerRejectsBlankTopic(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	brokerURL, cleanup := launchPeerMessageBroker(t)
	defer cleanup()

	s, err := OpenPeerMessageSession(ctx, PeerMessageConfig{BrokerURL: brokerURL})
	if err != nil {
		t.Fatalf("OpenPeerMessageSession() error = %v, want nil", err)
	}
	defer s.Close()

	fx := mustPeerMessageFixture(t)
	_, err = s.PublishInner(ctx, " ", fx.inner, fx.recipientX25519Pub, peere2e.SealOptions{})
	if err == nil {
		t.Fatalf("PublishInner(blank topic) error = nil, want non-nil")
	}
}

func TestPeerMessageSession_WaitOpenedReturnsBrokerErrors(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	s := &PeerMessageSession{
		msgCh: make(chan *packet.Message, 1),
		errCh: make(chan error, 1),
	}
	s.errCh <- io.ErrClosedPipe

	_, err := s.WaitOpened(ctx, mustPeerMessageFixture(t).recipientX25519Priv, peere2e.OpenOptions{})
	if !errors.Is(err, io.ErrClosedPipe) {
		t.Fatalf("WaitOpened() error = %v, want %v", err, io.ErrClosedPipe)
	}
}
