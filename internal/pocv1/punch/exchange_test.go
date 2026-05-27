package punch

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"errors"
	"net"
	"testing"

	"github.com/miopunch/miopunch/internal/pocv1/enroll"
	"github.com/miopunch/miopunch/internal/pocv1/peere2e"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/presence"
	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
	signalmqtt "github.com/miopunch/miopunch/internal/signaling/mqtt"
	"github.com/miopunch/miopunch/internal/udpowner"
)

func TestVerifyOfferRejectsWrongInnerKind(t *testing.T) {
	fx := mustExchangeFixture(t)
	opened := fx.offerOpened
	opened.Inner.Kind = pocwire.KindDialAnswer

	_, _, err := verifyOffer(fx.cfg, opened)
	if !errors.Is(err, ErrInvalidOffer) {
		t.Fatalf("verifyOffer(wrong inner kind) error = %v, want %v", err, ErrInvalidOffer)
	}
}

func TestVerifyOfferRejectsMalformedBody(t *testing.T) {
	fx := mustExchangeFixture(t)
	opened := fx.offerOpened
	opened.Inner.Body = []byte{0x01, 0x02}

	_, _, err := verifyOffer(fx.cfg, opened)
	if !errors.Is(err, pocwire.ErrTruncatedTLV) {
		t.Fatalf("verifyOffer(malformed body) error = %v, want %v", err, pocwire.ErrTruncatedTLV)
	}
}

func TestVerifyAnswerRejectsWrongInnerKind(t *testing.T) {
	fx := mustExchangeFixture(t)
	opened := fx.answerOpened
	opened.Inner.Kind = pocwire.KindDialOffer

	_, _, err := verifyAnswer(fx.cfg, opened, fx.offer.DialID, fx.offer.PunchToken, fx.offerMsgID)
	if !errors.Is(err, ErrInvalidAnswer) {
		t.Fatalf("verifyAnswer(wrong inner kind) error = %v, want %v", err, ErrInvalidAnswer)
	}
}

func TestVerifyAnswerRejectsDialIDMismatch(t *testing.T) {
	fx := mustExchangeFixture(t)
	_, _, err := verifyAnswer(fx.cfg, fx.answerOpened, mustCanonicalMsgID(t, "MFRGGZDFMZTWQ2LKNNWG23TPOI"), fx.offer.PunchToken, fx.offerMsgID)
	if !errors.Is(err, ErrInvalidAnswer) {
		t.Fatalf("verifyAnswer(dial_id mismatch) error = %v, want %v", err, ErrInvalidAnswer)
	}
}

func TestVerifyAnswerRejectsPunchTokenMismatch(t *testing.T) {
	fx := mustExchangeFixture(t)
	_, _, err := verifyAnswer(fx.cfg, fx.answerOpened, fx.offer.DialID, bytes.Repeat([]byte{0x21}, 16), fx.offerMsgID)
	if !errors.Is(err, ErrInvalidAnswer) {
		t.Fatalf("verifyAnswer(punch token mismatch) error = %v, want %v", err, ErrInvalidAnswer)
	}
}

func TestVerifyAnswerRejectsInReplyToMismatch(t *testing.T) {
	fx := mustExchangeFixture(t)
	_, _, err := verifyAnswer(fx.cfg, fx.answerOpened, fx.offer.DialID, fx.offer.PunchToken, mustMsgIDFromSeed(t, 0x39))
	if !errors.Is(err, ErrInvalidAnswer) {
		t.Fatalf("verifyAnswer(in_reply_to mismatch) error = %v, want %v", err, ErrInvalidAnswer)
	}
}

func TestExchangeOfferIgnoresUnrelatedAnswersThenReturnsMatchingAnswer(t *testing.T) {
	fx := mustExchangeFixture(t)
	session := &fakePeerMessageSession{
		waitOpened: []signalmqtt.OpenedPeerMessage{
			fx.unrelatedAnswerOpened,
			fx.answerOpened,
		},
	}

	answer, remote, msgID, err := exchangeOffer(context.Background(), fx.cfg, session, fx.remoteTrusted, fx.offer)
	if err != nil {
		t.Fatalf("exchangeOffer() error = %v, want nil", err)
	}
	if answer.DialID != fx.answer.DialID {
		t.Fatalf("exchangeOffer().DialID = %q, want %q", answer.DialID, fx.answer.DialID)
	}
	if remote.PeerID != fx.remoteTrusted.PeerID {
		t.Fatalf("exchangeOffer().remote.PeerID = %q, want %q", remote.PeerID, fx.remoteTrusted.PeerID)
	}
	if msgID == "" {
		t.Fatalf("exchangeOffer().msgID = %q, want non-empty", msgID)
	}
	if len(session.published) != 1 {
		t.Fatalf("exchangeOffer() published count = %d, want 1", len(session.published))
	}
	if session.published[0].topic != fx.remoteInboxTopic {
		t.Fatalf("exchangeOffer() published topic = %q, want %q", session.published[0].topic, fx.remoteInboxTopic)
	}
}

func TestExchangeOfferRejectsAnswerFromNonTargetPeer(t *testing.T) {
	fx := mustExchangeFixture(t)
	session := &fakePeerMessageSession{
		waitOpened: []signalmqtt.OpenedPeerMessage{fx.nonTargetAnswerOpened},
	}

	_, _, _, err := exchangeOffer(context.Background(), fx.cfg, session, fx.remoteTrusted, fx.offer)
	if !errors.Is(err, ErrInvalidAnswer) {
		t.Fatalf("exchangeOffer(non-target answer) error = %v, want %v", err, ErrInvalidAnswer)
	}
}

func TestWaitAndAnswerOfferIgnoresInvalidOfferBeforeValidOne(t *testing.T) {
	fx := mustExchangeFixture(t)
	conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("net.ListenUDP() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = conn.Close() })

	localCandidate := Candidate{Kind: CandidateKindHost, Addr: conn.LocalAddr().String()}
	cfg := fx.cfg
	cfg.UDPConn = conn
	cfg.LocalCandidates = []Candidate{localCandidate}
	cfg.AttemptConcurrency = 1
	cfg.AttemptPair = func(ctx context.Context, demux *udpowner.TraversalDemux, plan pairPlan, key []byte) (*net.UDPAddr, error) {
		return mustUDPAddr(t, plan.remote.Addr), nil
	}
	session := &fakePeerMessageSession{
		waitOpened: []signalmqtt.OpenedPeerMessage{
			fx.invalidOfferOpened,
			fx.offerOpened,
		},
	}

	got, err := waitAndAnswerOffer(context.Background(), cfg, session)
	if err != nil {
		t.Fatalf("waitAndAnswerOffer() error = %v, want nil", err)
	}
	if len(session.published) != 1 {
		t.Fatalf("waitAndAnswerOffer() published count = %d, want 1", len(session.published))
	}
	published := session.published[0]
	if published.topic != fx.remoteInboxTopic {
		t.Fatalf("waitAndAnswerOffer() published topic = %q, want %q", published.topic, fx.remoteInboxTopic)
	}
	if published.inner.Kind != pocwire.KindDialAnswer {
		t.Fatalf("waitAndAnswerOffer() published kind = %q, want %q", published.inner.Kind, pocwire.KindDialAnswer)
	}
	if published.inner.InReplyTo != fx.offerOpened.Inner.MsgID {
		t.Fatalf("waitAndAnswerOffer() published in_reply_to = %q, want %q", published.inner.InReplyTo, fx.offerOpened.Inner.MsgID)
	}
	var answer DialAnswer
	answer, err = UnmarshalDialAnswer(published.inner.Body)
	if err != nil {
		t.Fatalf("UnmarshalDialAnswer(published body) error = %v, want nil", err)
	}
	if answer.DialID != fx.offer.DialID {
		t.Fatalf("waitAndAnswerOffer() published dial_id = %q, want %q", answer.DialID, fx.offer.DialID)
	}
	if !bytes.Equal(answer.PunchToken, fx.offer.PunchToken) {
		t.Fatalf("waitAndAnswerOffer() published punch_token = %x, want %x", answer.PunchToken, fx.offer.PunchToken)
	}
	if got.RemoteIdentity.PeerID != fx.remoteTrusted.PeerID {
		t.Fatalf("waitAndAnswerOffer().RemoteIdentity.PeerID = %q, want %q", got.RemoteIdentity.PeerID, fx.remoteTrusted.PeerID)
	}
}

func TestWaitAndAnswerOfferReloadsRosterSnapshotForNewPeer(t *testing.T) {
	t.Parallel()

	fx := mustExchangeFixture(t)
	conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("net.ListenUDP() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = conn.Close() })

	cfg := fx.cfg
	cfg.UDPConn = conn
	cfg.LocalCandidates = []Candidate{{Kind: CandidateKindHost, Addr: conn.LocalAddr().String()}}
	cfg.AttemptConcurrency = 1
	cfg.AttemptPair = func(ctx context.Context, demux *udpowner.TraversalDemux, plan pairPlan, key []byte) (*net.UDPAddr, error) {
		return mustUDPAddr(t, plan.remote.Addr), nil
	}

	delete(cfg.TrustedRosterByID, fx.remoteTrusted.PeerID)
	cfg.RosterSnapshot = persist.RosterSnapshot{}
	if err := cfg.Store.ReplaceRosterSnapshot(cfg.NetworkID, persist.RosterSnapshot{}); err != nil {
		t.Fatalf("ReplaceRosterSnapshot(empty) error = %v, want nil", err)
	}
	if err := cfg.Store.ReplaceRosterSnapshot(cfg.NetworkID, persist.RosterSnapshot{
		Entries: []persist.RosterEntry{
			{PeerID: fx.remoteTrusted.PeerID, MemberCredential: fx.remoteTrusted.MemberCredential},
		},
	}); err != nil {
		t.Fatalf("ReplaceRosterSnapshot(updated) error = %v, want nil", err)
	}

	session := &fakePeerMessageSession{
		waitOpened: []signalmqtt.OpenedPeerMessage{fx.offerOpened},
	}

	got, err := waitAndAnswerOffer(context.Background(), cfg, session)
	if err != nil {
		t.Fatalf("waitAndAnswerOffer(reload roster) error = %v, want nil", err)
	}
	if got.RemoteIdentity.PeerID != fx.remoteTrusted.PeerID {
		t.Fatalf("waitAndAnswerOffer(reload roster).RemoteIdentity.PeerID = %q, want %q", got.RemoteIdentity.PeerID, fx.remoteTrusted.PeerID)
	}
	if len(session.published) != 1 {
		t.Fatalf("waitAndAnswerOffer(reload roster) published count = %d, want 1", len(session.published))
	}
}

type fakePeerMessageSession struct {
	waitOpened []signalmqtt.OpenedPeerMessage
	waitErr    error
	published  []publishedPeerMessage
}

type publishedPeerMessage struct {
	topic                 string
	inner                 pocwire.InnerMessage
	recipientX25519PubKey []byte
}

func (s *fakePeerMessageSession) Close() error { return nil }

func (s *fakePeerMessageSession) PublishInner(
	ctx context.Context,
	topic string,
	inner pocwire.InnerMessage,
	recipientX25519PublicKey []byte,
	opts peere2e.SealOptions,
) (pocwire.OuterHeader, error) {
	s.published = append(s.published, publishedPeerMessage{
		topic:                 topic,
		inner:                 inner,
		recipientX25519PubKey: append([]byte(nil), recipientX25519PublicKey...),
	})
	return pocwire.OuterHeader{}, nil
}

func (s *fakePeerMessageSession) WaitOpened(
	ctx context.Context,
	recipientX25519PrivateKey []byte,
	opts peere2e.OpenOptions,
) (signalmqtt.OpenedPeerMessage, error) {
	if len(s.waitOpened) == 0 {
		if s.waitErr != nil {
			return signalmqtt.OpenedPeerMessage{}, s.waitErr
		}
		return signalmqtt.OpenedPeerMessage{}, context.DeadlineExceeded
	}
	msg := s.waitOpened[0]
	s.waitOpened = s.waitOpened[1:]
	return msg, nil
}

type exchangeFixture struct {
	cfg                   LoadedConfig
	offer                 DialOffer
	answer                DialAnswer
	remoteTrusted         trustedRemote
	offerOpened           signalmqtt.OpenedPeerMessage
	answerOpened          signalmqtt.OpenedPeerMessage
	unrelatedAnswerOpened signalmqtt.OpenedPeerMessage
	nonTargetAnswerOpened signalmqtt.OpenedPeerMessage
	invalidOfferOpened    signalmqtt.OpenedPeerMessage
	offerMsgID            string
	remoteInboxTopic      string
}

func mustExchangeFixture(t *testing.T) exchangeFixture {
	t.Helper()

	authority := mustTestSigner(t, 0x11)
	networkID := mustCanonicalNetworkID(t, "LJMVUWK2LJMVUWK2LJMVUWK2LI")
	offerMsgID := mustMsgIDFromSeed(t, 0x30)
	localStore, localSigned := mustJoinedStoreFixture(t, authority.PrivateKey, networkID, 0x21)
	remoteSigned := mustSignedMemberCredential(t, authority.PrivateKey, networkID, 0x22)
	otherSigned := mustSignedMemberCredential(t, authority.PrivateKey, networkID, 0x23)
	if err := localStore.PersistJoinedBootstrap(persist.JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: localSigned.Raw,
		MailboxSecret:        bytes.Repeat([]byte{0x44}, 32),
		RuntimeBroker:        persist.RuntimeBroker{Endpoint: "tcp://127.0.0.1:1883"},
		RosterSnapshot: persist.RosterSnapshot{
			Entries: []persist.RosterEntry{
				{PeerID: remoteSigned.Signer.PeerID, MemberCredential: remoteSigned.Raw},
				{PeerID: otherSigned.Signer.PeerID, MemberCredential: otherSigned.Raw},
			},
		},
	}); err != nil {
		t.Fatalf("PersistJoinedBootstrap() error = %v, want nil", err)
	}
	loaded, err := loadConfig(Config{
		NetworkID:           networkID,
		AuthorityEd25519Pub: authority.PublicKey,
		Store:               localStore,
		Discover: presence.DiscoverProjection{
			Peers: []presence.DiscoverProjectionPeer{
				{PeerID: remoteSigned.Signer.PeerID, OnlineState: presence.OnlineStateOnline},
				{PeerID: otherSigned.Signer.PeerID, OnlineState: presence.OnlineStateOnline},
			},
		},
		LocalCandidates: []Candidate{{Kind: CandidateKindHost, Addr: "127.0.0.1:4001"}},
		UDPConn:         mustListenUDP(t),
		NewMsgID: func() (string, error) {
			return offerMsgID, nil
		},
		NewDialID: func() (string, error) {
			return mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA"), nil
		},
		NewPunchToken: func() ([]byte, error) {
			return bytes.Repeat([]byte{0x42}, 16), nil
		},
	})
	if err != nil {
		t.Fatalf("loadConfig() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = loaded.UDPConn.Close() })

	offer := DialOffer{
		DialID:           mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA"),
		PunchToken:       bytes.Repeat([]byte{0x42}, 16),
		Candidates:       []Candidate{{Kind: CandidateKindHost, Addr: "127.0.0.1:4101"}},
		MemberCredential: localSigned.Raw,
	}
	answer := DialAnswer{
		DialID:           offer.DialID,
		PunchToken:       append([]byte(nil), offer.PunchToken...),
		Candidates:       []Candidate{{Kind: CandidateKindHost, Addr: "127.0.0.1:4102"}},
		MemberCredential: remoteSigned.Raw,
	}
	inboundOffer := DialOffer{
		DialID:           offer.DialID,
		PunchToken:       append([]byte(nil), offer.PunchToken...),
		Candidates:       append([]Candidate(nil), answer.Candidates...),
		MemberCredential: remoteSigned.Raw,
	}
	remoteInboxTopic, err := loaded.TopicScope.InboxTopic(remoteSigned.Signer.PeerID)
	if err != nil {
		t.Fatalf("InboxTopic(remote) error = %v, want nil", err)
	}
	otherInboxTopic, err := loaded.TopicScope.InboxTopic(otherSigned.Signer.PeerID)
	if err != nil {
		t.Fatalf("InboxTopic(other) error = %v, want nil", err)
	}

	remoteTrusted, err := trustedRemoteFromRoster(loaded, loaded.TrustedRosterByID[remoteSigned.Signer.PeerID])
	if err != nil {
		t.Fatalf("trustedRemoteFromRoster(remote) error = %v, want nil", err)
	}

	return exchangeFixture{
		cfg:                   loaded,
		offer:                 offer,
		answer:                answer,
		remoteTrusted:         remoteTrusted,
		offerOpened:           mustOpenedDialMessage(t, remoteSigned.Signer.PrivateKey, remoteInboxTopic, loaded.LocalPeerID, offerMsgID, "", pocwire.KindDialOffer, inboundOffer),
		answerOpened:          mustOpenedDialMessage(t, remoteSigned.Signer.PrivateKey, remoteInboxTopic, loaded.LocalPeerID, mustMsgIDFromSeed(t, 0x31), offerMsgID, pocwire.KindDialAnswer, answer),
		unrelatedAnswerOpened: mustOpenedDialMessage(t, remoteSigned.Signer.PrivateKey, remoteInboxTopic, loaded.LocalPeerID, mustMsgIDFromSeed(t, 0x32), mustMsgIDFromSeed(t, 0x33), pocwire.KindDialAnswer, answer),
		nonTargetAnswerOpened: mustOpenedDialMessage(t, otherSigned.Signer.PrivateKey, otherInboxTopic, loaded.LocalPeerID, mustMsgIDFromSeed(t, 0x34), offerMsgID, pocwire.KindDialAnswer, DialAnswer{
			DialID:           offer.DialID,
			PunchToken:       append([]byte(nil), offer.PunchToken...),
			Candidates:       []Candidate{{Kind: CandidateKindHost, Addr: "127.0.0.1:4103"}},
			MemberCredential: otherSigned.Raw,
		}),
		invalidOfferOpened: mustOpenedDialMessage(t, remoteSigned.Signer.PrivateKey, remoteInboxTopic, loaded.LocalPeerID, mustMsgIDFromSeed(t, 0x35), "", pocwire.KindDialOffer, []byte{0x01, 0x02}),
		offerMsgID:         offerMsgID,
		remoteInboxTopic:   remoteInboxTopic,
	}
}

func mustJoinedStoreFixture(
	t *testing.T,
	authorityPriv ed25519.PrivateKey,
	networkID string,
	subjectSeed byte,
) (*persist.Store, testSignedCredential) {
	t.Helper()

	store, err := persist.Open(t.TempDir())
	if err != nil {
		t.Fatalf("persist.Open() error = %v, want nil", err)
	}
	keys, err := store.EnsureDeviceKeys()
	if err != nil {
		t.Fatalf("EnsureDeviceKeys() error = %v, want nil", err)
	}
	subjectPub, err := keys.Ed25519PublicKey()
	if err != nil {
		t.Fatalf("DeviceKeys.Ed25519PublicKey() error = %v, want nil", err)
	}
	subjectX25519, err := keys.X25519PublicKey()
	if err != nil {
		t.Fatalf("DeviceKeys.X25519PublicKey() error = %v, want nil", err)
	}
	credential := enroll.MemberCredential{
		NetworkID:         networkID,
		SubjectEd25519Pub: subjectPub,
		SubjectX25519Pub:  subjectX25519,
		Role:              "member",
		NotBeforeUnixMs:   1,
		NotAfterUnixMs:    2,
		IssuerKeyID:       "authority-01",
	}
	if err := enroll.SignMemberCredential(authorityPriv, &credential); err != nil {
		t.Fatalf("SignMemberCredential() error = %v, want nil", err)
	}
	raw, err := credential.MarshalBinary()
	if err != nil {
		t.Fatalf("MemberCredential.MarshalBinary() error = %v, want nil", err)
	}
	peerID, err := credential.PeerID()
	if err != nil {
		t.Fatalf("MemberCredential.PeerID() error = %v, want nil", err)
	}
	signer := mustTestSigner(t, subjectSeed)
	return store, testSignedCredential{
		Signer: testSigner{
			PrivateKey: signer.PrivateKey,
			PublicKey:  append([]byte(nil), subjectPub...),
			PeerID:     peerID,
		},
		Credential: credential,
		Raw:        raw,
	}
}

func mustOpenedDialMessage(
	t *testing.T,
	signerPriv ed25519.PrivateKey,
	topic string,
	dstPeerID string,
	msgID string,
	inReplyTo string,
	kind string,
	payload any,
) signalmqtt.OpenedPeerMessage {
	t.Helper()

	var body []byte
	var err error
	switch v := payload.(type) {
	case DialOffer:
		body, err = v.MarshalBinary()
	case DialAnswer:
		body, err = v.MarshalBinary()
	case []byte:
		body = append([]byte(nil), v...)
	default:
		t.Fatalf("mustOpenedDialMessage(payload=%T) unsupported payload type", payload)
	}
	if err != nil {
		t.Fatalf("mustOpenedDialMessage(%q) marshal error = %v, want nil", kind, err)
	}
	inner := pocwire.InnerMessage{
		DstPeerID:       dstPeerID,
		MsgID:           msgID,
		CreatedAtUnixMs: 1,
		ExpiresAtUnixMs: 2,
		Kind:            kind,
		InReplyTo:       inReplyTo,
		Body:            body,
	}
	if err := pocwire.SignInner(signerPriv, &inner); err != nil {
		t.Fatalf("SignInner(%q) error = %v, want nil", kind, err)
	}
	return signalmqtt.OpenedPeerMessage{
		Topic: topic,
		Inner: inner,
	}
}

func mustListenUDP(t *testing.T) *net.UDPConn {
	t.Helper()

	conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("net.ListenUDP() error = %v, want nil", err)
	}
	return conn
}

func mustUDPAddr(t *testing.T, value string) *net.UDPAddr {
	t.Helper()

	addr, err := net.ResolveUDPAddr("udp", value)
	if err != nil {
		t.Fatalf("ResolveUDPAddr(%q) error = %v, want nil", value, err)
	}
	return addr
}
