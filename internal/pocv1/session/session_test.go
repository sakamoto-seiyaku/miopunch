// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

package session

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"errors"
	"io"
	"net"
	"testing"
	"time"

	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/pocv1/enroll"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/punch"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

func TestPathResultUpgradePreservesStreamOpenEnvelope(t *testing.T) {
	t.Parallel()

	fx := mustSessionFixture(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)

	serverCh := make(chan sessionResult, 1)
	go func() {
		sess, err := Accept(ctx, fx.responderConfig(), fx.responderPath())
		serverCh <- sessionResult{session: sess, err: err}
	}()

	clientSess, err := Dial(ctx, fx.dialerConfig(), fx.dialerPath())
	if err != nil {
		t.Fatalf("Dial() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = clientSess.Close(dataplane.CloseReasonDaemonShutdown) })

	serverRes := <-serverCh
	if serverRes.err != nil {
		t.Fatalf("Accept() error = %v, want nil", serverRes.err)
	}
	t.Cleanup(func() { _ = serverRes.session.Close(dataplane.CloseReasonDaemonShutdown) })

	acceptedCh := make(chan *dataplane.AcceptedStream, 1)
	errCh := make(chan error, 1)
	go func() {
		accepted, err := serverRes.session.AcceptStream(ctx)
		if err != nil {
			errCh <- err
			return
		}
		acceptedCh <- accepted
	}()

	open := StreamOpen{
		Kind: dataplane.StreamKindShellV0,
		Metadata: map[string]string{
			"peer_id": fx.responderPeerID,
			"op":      "ping",
			"trace":   "trace-01",
		},
	}
	stream, err := clientSess.OpenStream(ctx, open)
	if err != nil {
		t.Fatalf("PeerSession.OpenStream() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = stream.Close() })

	var accepted *dataplane.AcceptedStream
	select {
	case accepted = <-acceptedCh:
	case err := <-errCh:
		t.Fatalf("PeerSession.AcceptStream() error = %v, want nil", err)
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for accepted stream")
	}
	if accepted == nil {
		t.Fatalf("accepted stream = nil, want non-nil")
	}
	t.Cleanup(func() { _ = accepted.Stream.Close() })
	if accepted.Open.Kind != open.Kind {
		t.Fatalf("AcceptedStream.Open.Kind = %q, want %q", accepted.Open.Kind, open.Kind)
	}
	if accepted.Open.Metadata["peer_id"] != open.Metadata["peer_id"] {
		t.Fatalf("AcceptedStream.Open.Metadata[peer_id] = %q, want %q", accepted.Open.Metadata["peer_id"], open.Metadata["peer_id"])
	}
	if accepted.Open.Metadata["trace"] != open.Metadata["trace"] {
		t.Fatalf("AcceptedStream.Open.Metadata[trace] = %q, want %q", accepted.Open.Metadata["trace"], open.Metadata["trace"])
	}

	payload := []byte("hello-session")
	if _, err := stream.Write(payload); err != nil {
		t.Fatalf("stream.Write() error = %v, want nil", err)
	}
	buf := make([]byte, len(payload))
	if _, err := io.ReadFull(accepted.Stream, buf); err != nil {
		t.Fatalf("io.ReadFull(accepted.Stream) error = %v, want nil", err)
	}
	if !bytes.Equal(buf, payload) {
		t.Fatalf("accepted stream payload = %q, want %q", string(buf), string(payload))
	}
	if got := dataplane.PathFactsFromSession(clientSess).SelectedPath; got != punch.PathDirectIPv4 {
		t.Fatalf("Dial() SessionPathFacts().SelectedPath = %q, want %q", got, punch.PathDirectIPv4)
	}
}

func TestDialRejectsPinMismatch(t *testing.T) {
	t.Parallel()

	fx := mustSessionFixture(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	t.Cleanup(cancel)

	serverCh := make(chan sessionResult, 1)
	go func() {
		sess, err := Accept(ctx, fx.responderConfig(), fx.responderPath())
		serverCh <- sessionResult{session: sess, err: err}
	}()

	_, badKeys := mustOpenStoreWithKeys(t)
	badRemoteCredential, _ := mustSignedCredential(t, fx.authorityPriv, fx.networkID, badKeys)
	badPath := fx.dialerPath()
	badPath.RemoteIdentity.MemberCredential = badRemoteCredential
	_, err := Dial(ctx, fx.dialerConfig(), badPath)
	if err == nil {
		t.Fatalf("Dial() error = nil, want pin mismatch")
	}
	_ = fx.responderConn.Close()
	serverRes := <-serverCh
	if serverRes.session != nil {
		_ = serverRes.session.Close(dataplane.CloseReasonDaemonShutdown)
	}
}

func TestDialRejectsInvalidRemoteCredential(t *testing.T) {
	t.Parallel()

	fx := mustSessionFixture(t)
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	t.Cleanup(cancel)

	path := fx.dialerPath()
	path.RemoteIdentity.MemberCredential = append([]byte(nil), fx.responderCredential...)
	path.RemoteIdentity.MemberCredential[len(path.RemoteIdentity.MemberCredential)-1] ^= 0x01

	_, err := Dial(ctx, fx.dialerConfig(), path)
	if err == nil {
		t.Fatalf("Dial() error = nil, want invalid credential")
	}
}

func TestDialRejectsLocalCredentialMismatch(t *testing.T) {
	t.Parallel()

	fx := mustSessionFixture(t)
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	t.Cleanup(cancel)

	mismatchStore, mismatchKeys := mustOpenStoreWithKeys(t)
	mismatchCredential, _ := mustSignedCredential(t, fx.authorityPriv, fx.networkID, mismatchKeys)
	if err := mismatchStore.PersistJoinedBootstrap(persist.JoinedBootstrap{
		NetworkID:            fx.networkID,
		SelfMemberCredential: mismatchCredential,
		MailboxSecret:        bytes.Repeat([]byte{0x44}, 32),
		RuntimeBroker:        persist.RuntimeBroker{Endpoint: "tcp://127.0.0.1:1883"},
		RosterSnapshot: persist.RosterSnapshot{
			Entries: []persist.RosterEntry{
				{PeerID: fx.responderPeerID, MemberCredential: fx.responderCredential},
			},
		},
	}); err != nil {
		t.Fatalf("PersistJoinedBootstrap(mismatch) error = %v, want nil", err)
	}

	cfg := fx.dialerConfig()
	cfg.Store = mismatchStore
	_, err := Dial(ctx, cfg, fx.dialerPath())
	if err == nil {
		t.Fatalf("Dial() error = nil, want local credential mismatch")
	}
}

func TestDialHandshakeFailsWhenPeerNeverAccepts(t *testing.T) {
	t.Parallel()

	fx := mustSessionFixture(t)
	ctx, cancel := context.WithTimeout(context.Background(), 300*time.Millisecond)
	t.Cleanup(cancel)

	_, err := Dial(ctx, fx.dialerConfig(), fx.dialerPath())
	if err == nil {
		t.Fatalf("Dial() error = nil, want handshake failure")
	}
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("Dial() error = %v, want wrapped context deadline exceeded", err)
	}
	assertUDPConnOpen(t, fx.dialerConn, "dialer UDPConn after failed Dial")
}

func TestPeerSessionCloseDoesNotCloseBorrowedUDPConns(t *testing.T) {
	t.Parallel()

	fx := mustSessionFixture(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)

	serverCh := make(chan sessionResult, 1)
	go func() {
		sess, err := Accept(ctx, fx.responderConfig(), fx.responderPath())
		serverCh <- sessionResult{session: sess, err: err}
	}()

	clientSess, err := Dial(ctx, fx.dialerConfig(), fx.dialerPath())
	if err != nil {
		t.Fatalf("Dial() error = %v, want nil", err)
	}
	serverRes := <-serverCh
	if serverRes.err != nil {
		t.Fatalf("Accept() error = %v, want nil", serverRes.err)
	}

	if err := clientSess.Close(dataplane.CloseReasonDaemonShutdown); err != nil {
		t.Fatalf("client PeerSession.Close() error = %v, want nil", err)
	}
	if err := serverRes.session.Close(dataplane.CloseReasonDaemonShutdown); err != nil {
		t.Fatalf("server PeerSession.Close() error = %v, want nil", err)
	}
	assertUDPConnOpen(t, fx.dialerConn, "dialer UDPConn after PeerSession.Close")
	assertUDPConnOpen(t, fx.responderConn, "responder UDPConn after PeerSession.Close")
}

type sessionFixture struct {
	authorityPriv       ed25519.PrivateKey
	authorityPub        ed25519.PublicKey
	networkID           string
	dialerStore         *persist.Store
	responderStore      *persist.Store
	dialerConn          *net.UDPConn
	responderConn       *net.UDPConn
	dialerCredential    []byte
	responderCredential []byte
	dialerPeerID        string
	responderPeerID     string
}

type sessionResult struct {
	session PeerSession
	err     error
}

func mustSessionFixture(t *testing.T) sessionFixture {
	t.Helper()

	authorityPriv := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{0x11}, ed25519.SeedSize))
	authorityPub := authorityPriv.Public().(ed25519.PublicKey)
	networkID, err := wire.EncodeNetworkID(bytes.Repeat([]byte{0x5a}, wire.RawIDLen))
	if err != nil {
		t.Fatalf("EncodeNetworkID() error = %v, want nil", err)
	}

	dialerStore, dialerSigned := mustOpenStoreWithKeys(t)
	responderStore, responderSigned := mustOpenStoreWithKeys(t)
	dialerCredential, dialerPeerID := mustSignedCredential(t, authorityPriv, networkID, dialerSigned)
	responderCredential, responderPeerID := mustSignedCredential(t, authorityPriv, networkID, responderSigned)

	if err := dialerStore.PersistJoinedBootstrap(persist.JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: dialerCredential,
		MailboxSecret:        bytes.Repeat([]byte{0x44}, 32),
		RuntimeBroker:        persist.RuntimeBroker{Endpoint: "tcp://127.0.0.1:1883"},
		RosterSnapshot: persist.RosterSnapshot{
			Entries: []persist.RosterEntry{
				{PeerID: responderPeerID, MemberCredential: responderCredential, DeviceName: "responder", Platform: "linux"},
			},
		},
	}); err != nil {
		t.Fatalf("PersistJoinedBootstrap(dialer) error = %v, want nil", err)
	}
	if err := responderStore.PersistJoinedBootstrap(persist.JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: responderCredential,
		MailboxSecret:        bytes.Repeat([]byte{0x44}, 32),
		RuntimeBroker:        persist.RuntimeBroker{Endpoint: "tcp://127.0.0.1:1883"},
		RosterSnapshot: persist.RosterSnapshot{
			Entries: []persist.RosterEntry{
				{PeerID: dialerPeerID, MemberCredential: dialerCredential, DeviceName: "dialer", Platform: "linux"},
			},
		},
	}); err != nil {
		t.Fatalf("PersistJoinedBootstrap(responder) error = %v, want nil", err)
	}

	dialerConn := mustListenUDP(t)
	responderConn := mustListenUDP(t)

	return sessionFixture{
		authorityPriv:       authorityPriv,
		authorityPub:        append(ed25519.PublicKey(nil), authorityPub...),
		networkID:           networkID,
		dialerStore:         dialerStore,
		responderStore:      responderStore,
		dialerConn:          dialerConn,
		responderConn:       responderConn,
		dialerCredential:    dialerCredential,
		responderCredential: responderCredential,
		dialerPeerID:        dialerPeerID,
		responderPeerID:     responderPeerID,
	}
}

func (f sessionFixture) dialerConfig() Config {
	return Config{
		NetworkID:           f.networkID,
		AuthorityEd25519Pub: f.authorityPub,
		Store:               f.dialerStore,
		IdleTimeout:         time.Minute,
	}
}

func (f sessionFixture) responderConfig() Config {
	return Config{
		NetworkID:           f.networkID,
		AuthorityEd25519Pub: f.authorityPub,
		Store:               f.responderStore,
		IdleTimeout:         time.Minute,
	}
}

func (f sessionFixture) dialerPath() punch.PathResult {
	return punch.PathResult{
		Conn:       f.dialerConn,
		RemoteAddr: udpAddrClone(f.responderConn.LocalAddr().(*net.UDPAddr)),
		RemoteIdentity: punch.TrustedRemoteIdentity{
			PeerID:           f.responderPeerID,
			MemberCredential: append([]byte(nil), f.responderCredential...),
		},
		Evidence: punch.PunchEvidence{SelectedPath: punch.PathDirectIPv4},
	}
}

func (f sessionFixture) responderPath() punch.PathResult {
	return punch.PathResult{
		Conn:       f.responderConn,
		RemoteAddr: udpAddrClone(f.dialerConn.LocalAddr().(*net.UDPAddr)),
		RemoteIdentity: punch.TrustedRemoteIdentity{
			PeerID:           f.dialerPeerID,
			MemberCredential: append([]byte(nil), f.dialerCredential...),
		},
		Evidence: punch.PunchEvidence{SelectedPath: punch.PathDirectIPv4},
	}
}

func mustOpenStoreWithKeys(t *testing.T) (*persist.Store, persist.DeviceKeys) {
	t.Helper()

	store, err := persist.Open(t.TempDir())
	if err != nil {
		t.Fatalf("persist.Open() error = %v, want nil", err)
	}
	keys, err := store.EnsureDeviceKeys()
	if err != nil {
		t.Fatalf("EnsureDeviceKeys() error = %v, want nil", err)
	}
	return store, keys
}

func mustSignedCredential(t *testing.T, authorityPriv ed25519.PrivateKey, networkID string, keys persist.DeviceKeys) ([]byte, string) {
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

func mustListenUDP(t *testing.T) *net.UDPConn {
	t.Helper()

	addr, err := net.ResolveUDPAddr("udp4", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("ResolveUDPAddr() error = %v, want nil", err)
	}
	conn, err := net.ListenUDP("udp4", addr)
	if err != nil {
		t.Fatalf("net.ListenUDP() error = %v, want nil", err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	return conn
}

func assertUDPConnOpen(t *testing.T, conn *net.UDPConn, label string) {
	t.Helper()

	receiver := mustListenUDP(t)
	if _, err := conn.WriteToUDP([]byte("still-open"), receiver.LocalAddr().(*net.UDPAddr)); err != nil {
		t.Fatalf("%s WriteToUDP() error = %v, want nil", label, err)
	}
}

func udpAddrClone(addr *net.UDPAddr) *net.UDPAddr {
	if addr == nil {
		return nil
	}
	return &net.UDPAddr{
		IP:   append(net.IP(nil), addr.IP...),
		Port: addr.Port,
		Zone: addr.Zone,
	}
}
