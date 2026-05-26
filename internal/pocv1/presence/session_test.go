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

package presence

import (
	"bytes"
	"context"
	"testing"
	"time"

	"github.com/256dpi/gomqtt/broker"
	"github.com/256dpi/gomqtt/transport"

	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

func TestSessionLocalMQTTSmokeHydratesRetainedGracefulOfflineAndReconnect(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	brokerEndpoint, cleanup := launchPresenceBroker(t)
	defer cleanup()

	cfgA, cfgB := mustConfigPair(t, brokerEndpoint)

	sessionA, err := OpenSession(ctx, cfgA)
	if err != nil {
		t.Fatalf("OpenSession(A) error = %v, want nil", err)
	}
	defer sessionA.Abort()

	time.Sleep(100 * time.Millisecond)

	sessionB, err := OpenSession(ctx, cfgB)
	if err != nil {
		t.Fatalf("OpenSession(B) error = %v, want nil", err)
	}
	defer sessionB.Abort()

	if err := waitForObservedPeer(ctx, sessionA, cfgA.SelfPeerID); err != nil {
		t.Fatalf("waitForObservedPeer(A self) error = %v, want nil", err)
	}
	assertSessionDiagnosticsEmpty(t, sessionA, "sessionA")
	if err := waitForObservedPeer(ctx, sessionB, cfgB.SelfPeerID); err != nil {
		t.Fatalf("waitForObservedPeer(B self) error = %v, want nil", err)
	}
	assertSessionDiagnosticsEmpty(t, sessionB, "sessionB")

	view, err := sessionB.WaitForPeerState(ctx, cfgA.SelfPeerID, OnlineStateOnline)
	if err != nil {
		t.Fatalf("WaitForPeerState(online) error = %v, want nil", err)
	}
	if len(view.Peers) != 1 {
		t.Fatalf("WaitForPeerState(online) peers length = %d, want 1", len(view.Peers))
	}

	if err := sessionA.Close(ctx); err != nil {
		t.Fatalf("Close(A) error = %v, want nil", err)
	}
	view, err = sessionB.WaitForPeerState(ctx, cfgA.SelfPeerID, OnlineStateOffline)
	if err != nil {
		t.Fatalf("WaitForPeerState(offline) error = %v, want nil", err)
	}
	if len(view.Peers) != 1 {
		t.Fatalf("WaitForPeerState(offline) peers length = %d, want 1", len(view.Peers))
	}

	sessionA2, err := OpenSession(ctx, cfgA)
	if err != nil {
		t.Fatalf("OpenSession(A reconnect) error = %v, want nil", err)
	}
	defer sessionA2.Abort()

	if err := waitForObservedPeer(ctx, sessionA2, cfgA.SelfPeerID); err != nil {
		t.Fatalf("waitForObservedPeer(A reconnect self) error = %v, want nil", err)
	}
	assertSessionDiagnosticsEmpty(t, sessionA2, "sessionA2")
	assertSessionDiagnosticsEmpty(t, sessionB, "sessionB after reconnect")

	view, err = sessionB.WaitForPeerState(ctx, cfgA.SelfPeerID, OnlineStateOnline)
	if err != nil {
		t.Fatalf("WaitForPeerState(reconnect online) error = %v, want nil", err)
	}
	if len(view.Peers) != 1 {
		t.Fatalf("WaitForPeerState(reconnect online) peers length = %d, want 1", len(view.Peers))
	}
}

func TestSessionUnexpectedDisconnectUsesRetainedWill(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	brokerEndpoint, cleanup := launchPresenceBroker(t)
	defer cleanup()

	cfgA, cfgB := mustConfigPair(t, brokerEndpoint)

	sessionA, err := OpenSession(ctx, cfgA)
	if err != nil {
		t.Fatalf("OpenSession(A) error = %v, want nil", err)
	}
	defer sessionA.Abort()

	sessionB, err := OpenSession(ctx, cfgB)
	if err != nil {
		t.Fatalf("OpenSession(B) error = %v, want nil", err)
	}
	defer sessionB.Abort()

	if _, err := sessionB.WaitForPeerState(ctx, cfgA.SelfPeerID, OnlineStateOnline); err != nil {
		t.Fatalf("WaitForPeerState(online) error = %v, want nil", err)
	}

	if err := sessionA.Abort(); err != nil {
		t.Fatalf("Abort(A) error = %v, want nil", err)
	}

	view, err := sessionB.WaitForPeerState(ctx, cfgA.SelfPeerID, OnlineStateOffline)
	if err != nil {
		t.Fatalf("WaitForPeerState(LWT offline) error = %v, want nil", err)
	}
	if len(view.Peers) != 1 {
		t.Fatalf("WaitForPeerState(LWT offline) peers length = %d, want 1", len(view.Peers))
	}
}

func TestSessionLateSubscriberHydratesGracefulOfflineFromRetainedSnapshot(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	brokerEndpoint, cleanup := launchPresenceBroker(t)
	defer cleanup()

	cfgA, cfgB := mustConfigPair(t, brokerEndpoint)

	sessionA, err := OpenSession(ctx, cfgA)
	if err != nil {
		t.Fatalf("OpenSession(A) error = %v, want nil", err)
	}
	if err := waitForObservedPeer(ctx, sessionA, cfgA.SelfPeerID); err != nil {
		t.Fatalf("waitForObservedPeer(A self) error = %v, want nil", err)
	}
	assertSessionDiagnosticsEmpty(t, sessionA, "sessionA before close")

	if err := sessionA.Close(ctx); err != nil {
		t.Fatalf("Close(A) error = %v, want nil", err)
	}

	sessionB, err := OpenSession(ctx, cfgB)
	if err != nil {
		t.Fatalf("OpenSession(B) error = %v, want nil", err)
	}
	defer sessionB.Abort()

	if err := waitForObservedPeer(ctx, sessionB, cfgA.SelfPeerID); err != nil {
		t.Fatalf("waitForObservedPeer(B sees A retained offline) error = %v, want nil", err)
	}
	assertSessionDiagnosticsEmpty(t, sessionB, "sessionB late subscriber")

	view, err := sessionB.WaitForPeerState(ctx, cfgA.SelfPeerID, OnlineStateOffline)
	if err != nil {
		t.Fatalf("WaitForPeerState(retained offline) error = %v, want nil", err)
	}
	if len(view.Peers) != 1 {
		t.Fatalf("WaitForPeerState(retained offline) peers length = %d, want 1", len(view.Peers))
	}
}

func launchPresenceBroker(t *testing.T) (string, func()) {
	t.Helper()

	server, err := transport.Launch("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("transport.Launch() error = %v, want nil", err)
	}
	backend := broker.NewMemoryBackend()
	engine := broker.NewEngine(backend)
	engine.Accept(server)
	return server.Addr().String(), func() {
		_ = server.Close()
		engine.Close()
	}
}

func waitForObservedPeer(ctx context.Context, s *Session, peerID string) error {
	ticker := time.NewTicker(10 * time.Millisecond)
	defer ticker.Stop()

	for {
		s.mu.Lock()
		_, ok := s.engine.observed[peerID]
		s.mu.Unlock()
		if ok {
			return nil
		}

		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
		}
	}
}

func assertSessionDiagnosticsEmpty(t *testing.T, s *Session, label string) {
	t.Helper()

	if got := s.Diagnostics(); len(got) != 0 {
		t.Fatalf("%s Diagnostics() = %+v, want empty", label, got)
	}
}

func mustConfigPair(t *testing.T, brokerEndpoint string) (Config, Config) {
	t.Helper()

	storeA, err := persist.Open(t.TempDir())
	if err != nil {
		t.Fatalf("persist.Open(A) error = %v, want nil", err)
	}
	storeB, err := persist.Open(t.TempDir())
	if err != nil {
		t.Fatalf("persist.Open(B) error = %v, want nil", err)
	}

	keysA, err := storeA.EnsureDeviceKeys()
	if err != nil {
		t.Fatalf("EnsureDeviceKeys(A) error = %v, want nil", err)
	}
	keysB, err := storeB.EnsureDeviceKeys()
	if err != nil {
		t.Fatalf("EnsureDeviceKeys(B) error = %v, want nil", err)
	}

	peerA, err := keysA.PeerID()
	if err != nil {
		t.Fatalf("DeviceKeys.PeerID(A) error = %v, want nil", err)
	}
	peerB, err := keysB.PeerID()
	if err != nil {
		t.Fatalf("DeviceKeys.PeerID(B) error = %v, want nil", err)
	}

	networkID, err := wire.EncodeNetworkID(bytes.Repeat([]byte{0x7a}, wire.RawIDLen))
	if err != nil {
		t.Fatalf("EncodeNetworkID() error = %v, want nil", err)
	}
	mailboxSecret := bytes.Repeat([]byte{0x44}, 32)

	joinedA := persist.JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: []byte("self-member-credential-a"),
		MailboxSecret:        append([]byte(nil), mailboxSecret...),
		RuntimeBroker: persist.RuntimeBroker{
			Endpoint: brokerEndpoint,
		},
		RosterSnapshot: persist.RosterSnapshot{
			Entries: []persist.RosterEntry{
				{
					PeerID:           peerB,
					MemberCredential: []byte("credential-b"),
					DeviceName:       "beta",
					Platform:         "windows",
				},
			},
		},
	}
	joinedB := persist.JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: []byte("self-member-credential-b"),
		MailboxSecret:        append([]byte(nil), mailboxSecret...),
		RuntimeBroker: persist.RuntimeBroker{
			Endpoint: brokerEndpoint,
		},
		RosterSnapshot: persist.RosterSnapshot{
			Entries: []persist.RosterEntry{
				{
					PeerID:           peerA,
					MemberCredential: []byte("credential-a"),
					DeviceName:       "alpha",
					Platform:         "linux",
				},
			},
		},
	}

	if err := storeA.PersistJoinedBootstrap(joinedA); err != nil {
		t.Fatalf("PersistJoinedBootstrap(A) error = %v, want nil", err)
	}
	if err := storeB.PersistJoinedBootstrap(joinedB); err != nil {
		t.Fatalf("PersistJoinedBootstrap(B) error = %v, want nil", err)
	}

	cfgA, err := LoadConfig(storeA, networkID, "alpha-local", "linux", "1.0.0")
	if err != nil {
		t.Fatalf("LoadConfig(A) error = %v, want nil", err)
	}
	cfgB, err := LoadConfig(storeB, networkID, "beta-local", "windows", "1.0.0")
	if err != nil {
		t.Fatalf("LoadConfig(B) error = %v, want nil", err)
	}
	return cfgA, cfgB
}
