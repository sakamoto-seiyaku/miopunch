package punch

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"errors"
	"net"
	"sync"
	"testing"
	"time"

	"github.com/256dpi/gomqtt/broker"
	"github.com/256dpi/gomqtt/transport"

	"github.com/miopunch/miopunch/internal/pocv1/enroll"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/presence"
	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
)

func TestDialAndHandleOneSmokeProduceRosterBackedPathResult(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	brokerURL, cleanup := launchPunchSmokeBroker(t)
	defer cleanup()

	fx := mustPunchSmokeFixture(t, brokerURL)
	ready := make(chan struct{})
	handleResultCh := make(chan PathResult, 1)
	handleErrCh := make(chan error, 1)

	go func() {
		got, err := HandleOne(ctx, Config{
			NetworkID:           fx.networkID,
			AuthorityEd25519Pub: fx.authorityPub,
			Store:               fx.responderStore,
			Discover: presence.DiscoverProjection{
				Peers: []presence.DiscoverProjectionPeer{
					{PeerID: fx.dialerPeerID, OnlineState: presence.OnlineStateOnline},
				},
			},
			LocalCandidates: []Candidate{
				{Kind: CandidateKindHost, Addr: fx.responderConn.LocalAddr().String()},
			},
			UDPConn:         fx.responderConn,
			AttemptBudget:   3 * time.Second,
			OpenPeerMessage: openerWithReady(ready),
		})
		if err != nil {
			handleErrCh <- err
			return
		}
		handleResultCh <- got
	}()

	select {
	case <-ready:
	case err := <-handleErrCh:
		t.Fatalf("HandleOne() early error = %v, want nil", err)
	case <-ctx.Done():
		t.Fatalf("timed out waiting for HandleOne() readiness: %v", ctx.Err())
	}

	dialResult, err := Dial(ctx, Config{
		NetworkID:           fx.networkID,
		AuthorityEd25519Pub: fx.authorityPub,
		Store:               fx.dialerStore,
		Discover: presence.DiscoverProjection{
			Peers: []presence.DiscoverProjectionPeer{
				{PeerID: fx.responderPeerID, OnlineState: presence.OnlineStateOnline},
			},
		},
		LocalCandidates: []Candidate{
			{Kind: CandidateKindHost, Addr: fx.dialerConn.LocalAddr().String()},
		},
		UDPConn:       fx.dialerConn,
		AttemptBudget: 3 * time.Second,
	}, Target{PeerID: fx.responderPeerID})
	if err != nil {
		t.Fatalf("Dial() error = %v, want nil", err)
	}

	var handleResult PathResult
	select {
	case handleResult = <-handleResultCh:
	case err := <-handleErrCh:
		t.Fatalf("HandleOne() error = %v, want nil", err)
	case <-ctx.Done():
		t.Fatalf("timed out waiting for HandleOne() result: %v", ctx.Err())
	}

	assertSmokePathResult(t, "Dial()", dialResult, fx.responderPeerID, fx.responderCredential, fx.dialerConn.LocalAddr().String(), fx.responderConn.LocalAddr().String())
	assertSmokePathResult(t, "HandleOne()", handleResult, fx.dialerPeerID, fx.dialerCredential, fx.responderConn.LocalAddr().String(), fx.dialerConn.LocalAddr().String())

	if dialResult.Evidence.DialID != handleResult.Evidence.DialID {
		t.Fatalf("Dial().Evidence.DialID = %q, HandleOne().Evidence.DialID = %q, want equal", dialResult.Evidence.DialID, handleResult.Evidence.DialID)
	}
	if err := dialResult.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
		t.Fatalf("Dial().Close() error = %v, want nil", err)
	}
	if err := handleResult.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
		t.Fatalf("HandleOne().Close() error = %v, want nil", err)
	}
}

func assertSmokePathResult(
	t *testing.T,
	label string,
	got PathResult,
	wantPeerID string,
	wantCredential []byte,
	wantLocalAddr string,
	wantRemoteAddr string,
) {
	t.Helper()

	if got.Conn == nil {
		t.Fatalf("%s Conn = nil, want non-nil", label)
	}
	if got.RemoteAddr == nil || got.RemoteAddr.String() != wantRemoteAddr {
		t.Fatalf("%s RemoteAddr = %#v, want %q", label, got.RemoteAddr, wantRemoteAddr)
	}
	if got.RemoteIdentity.PeerID != wantPeerID {
		t.Fatalf("%s RemoteIdentity.PeerID = %q, want %q", label, got.RemoteIdentity.PeerID, wantPeerID)
	}
	if !bytes.Equal(got.RemoteIdentity.MemberCredential, wantCredential) {
		t.Fatalf("%s RemoteIdentity.MemberCredential mismatch", label)
	}
	if len(got.Evidence.AttemptedPairs) != 1 {
		t.Fatalf("%s Evidence.AttemptedPairs length = %d, want 1", label, len(got.Evidence.AttemptedPairs))
	}
	if got.Evidence.AttemptedPairs[0].Result != "selected" {
		t.Fatalf("%s Evidence.AttemptedPairs[0].Result = %q, want %q", label, got.Evidence.AttemptedPairs[0].Result, "selected")
	}
	if got.Evidence.SelectedLocal.Addr != wantLocalAddr {
		t.Fatalf("%s Evidence.SelectedLocal.Addr = %q, want %q", label, got.Evidence.SelectedLocal.Addr, wantLocalAddr)
	}
	if got.Evidence.SelectedRemote.Addr != wantRemoteAddr {
		t.Fatalf("%s Evidence.SelectedRemote.Addr = %q, want %q", label, got.Evidence.SelectedRemote.Addr, wantRemoteAddr)
	}
	if got.Evidence.SelectedRemoteUDP != wantRemoteAddr {
		t.Fatalf("%s Evidence.SelectedRemoteUDP = %q, want %q", label, got.Evidence.SelectedRemoteUDP, wantRemoteAddr)
	}
	if got.Evidence.SelectedPath != PathDirectIPv4 {
		t.Fatalf("%s Evidence.SelectedPath = %q, want %q", label, got.Evidence.SelectedPath, PathDirectIPv4)
	}
}

func launchPunchSmokeBroker(t *testing.T) (string, func()) {
	t.Helper()

	server, err := transport.Launch("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("transport.Launch() error = %v, want nil", err)
	}
	backend := broker.NewMemoryBackend()
	engine := broker.NewEngine(backend)
	engine.Accept(server)
	return "tcp://" + server.Addr().String(), func() {
		_ = server.Close()
		engine.Close()
	}
}

func openerWithReady(ready chan<- struct{}) peerMessageOpener {
	var once sync.Once

	return func(ctx context.Context, cfg LoadedConfig) (peerMessageSession, error) {
		session, err := defaultPeerMessageOpener(ctx, cfg)
		if err != nil {
			return nil, err
		}
		once.Do(func() { close(ready) })
		return session, nil
	}
}

type punchSmokeFixture struct {
	authorityPub        ed25519.PublicKey
	networkID           string
	dialerPeerID        string
	responderPeerID     string
	dialerCredential    []byte
	responderCredential []byte
	dialerStore         *persist.Store
	responderStore      *persist.Store
	dialerConn          *net.UDPConn
	responderConn       *net.UDPConn
}

func mustPunchSmokeFixture(t *testing.T, brokerURL string) punchSmokeFixture {
	t.Helper()

	authoritySeed := bytes.Repeat([]byte{0x61}, ed25519.SeedSize)
	authorityPriv := ed25519.NewKeyFromSeed(authoritySeed)
	authorityPub := authorityPriv.Public().(ed25519.PublicKey)
	networkID, err := pocwire.EncodeNetworkID(bytes.Repeat([]byte{0x7a}, pocwire.RawIDLen))
	if err != nil {
		t.Fatalf("EncodeNetworkID() error = %v, want nil", err)
	}
	mailboxSecret := bytes.Repeat([]byte{0x44}, 32)

	dialerStore, err := persist.Open(t.TempDir())
	if err != nil {
		t.Fatalf("persist.Open(dialer) error = %v, want nil", err)
	}
	responderStore, err := persist.Open(t.TempDir())
	if err != nil {
		t.Fatalf("persist.Open(responder) error = %v, want nil", err)
	}

	dialerKeys, err := dialerStore.EnsureDeviceKeys()
	if err != nil {
		t.Fatalf("EnsureDeviceKeys(dialer) error = %v, want nil", err)
	}
	responderKeys, err := responderStore.EnsureDeviceKeys()
	if err != nil {
		t.Fatalf("EnsureDeviceKeys(responder) error = %v, want nil", err)
	}

	dialerCredential, dialerPeerID := mustSignedCredential(t, authorityPriv, networkID, dialerKeys)
	responderCredential, responderPeerID := mustSignedCredential(t, authorityPriv, networkID, responderKeys)

	if err := dialerStore.PersistJoinedBootstrap(persist.JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: dialerCredential,
		MailboxSecret:        append([]byte(nil), mailboxSecret...),
		RuntimeBroker:        persist.RuntimeBroker{Endpoint: brokerURL},
		RosterSnapshot: persist.RosterSnapshot{
			Entries: []persist.RosterEntry{
				{
					PeerID:           responderPeerID,
					MemberCredential: responderCredential,
					DeviceName:       "responder",
					Platform:         "linux",
				},
			},
		},
	}); err != nil {
		t.Fatalf("PersistJoinedBootstrap(dialer) error = %v, want nil", err)
	}
	if err := responderStore.PersistJoinedBootstrap(persist.JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: responderCredential,
		MailboxSecret:        append([]byte(nil), mailboxSecret...),
		RuntimeBroker:        persist.RuntimeBroker{Endpoint: brokerURL},
		RosterSnapshot: persist.RosterSnapshot{
			Entries: []persist.RosterEntry{
				{
					PeerID:           dialerPeerID,
					MemberCredential: dialerCredential,
					DeviceName:       "dialer",
					Platform:         "linux",
				},
			},
		},
	}); err != nil {
		t.Fatalf("PersistJoinedBootstrap(responder) error = %v, want nil", err)
	}

	dialerConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("net.ListenUDP(dialer) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = dialerConn.Close() })
	responderConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 0})
	if err != nil {
		t.Fatalf("net.ListenUDP(responder) error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = responderConn.Close() })

	return punchSmokeFixture{
		authorityPub:        authorityPub,
		networkID:           networkID,
		dialerPeerID:        dialerPeerID,
		responderPeerID:     responderPeerID,
		dialerCredential:    dialerCredential,
		responderCredential: responderCredential,
		dialerStore:         dialerStore,
		responderStore:      responderStore,
		dialerConn:          dialerConn,
		responderConn:       responderConn,
	}
}

func mustSignedCredential(
	t *testing.T,
	authorityPriv ed25519.PrivateKey,
	networkID string,
	keys persist.DeviceKeys,
) ([]byte, string) {
	t.Helper()

	subjectEd25519Pub, err := keys.Ed25519PublicKey()
	if err != nil {
		t.Fatalf("DeviceKeys.Ed25519PublicKey() error = %v, want nil", err)
	}
	subjectX25519Pub, err := keys.X25519PublicKey()
	if err != nil {
		t.Fatalf("DeviceKeys.X25519PublicKey() error = %v, want nil", err)
	}
	credential := enroll.MemberCredential{
		NetworkID:         networkID,
		SubjectEd25519Pub: subjectEd25519Pub,
		SubjectX25519Pub:  subjectX25519Pub,
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
	return raw, peerID
}
