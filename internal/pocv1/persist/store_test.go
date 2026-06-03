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

package persist

import (
	"bytes"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

func TestOpenCreatesMinimalFirstRunLayout(t *testing.T) {
	t.Parallel()

	root := filepath.Join(t.TempDir(), "state")
	store, err := Open(root)
	if err != nil {
		t.Fatalf("Open(%q) error = %v, want nil", root, err)
	}

	if store.root != root {
		t.Fatalf("Open(%q) root = %q, want %q", root, store.root, root)
	}
	assertDirExists(t, root)
	assertDirExists(t, filepath.Join(root, "device"))
	assertPathMissing(t, filepath.Join(root, "networks"))
	assertMode(t, root, dirPerm)
	assertMode(t, filepath.Join(root, "device"), dirPerm)
}

func TestEnsureDeviceKeysRoundTripAndPeerID(t *testing.T) {
	t.Parallel()

	store := mustOpenStore(t)
	first, err := store.EnsureDeviceKeys()
	if err != nil {
		t.Fatalf("EnsureDeviceKeys() error = %v, want nil", err)
	}
	second, err := store.EnsureDeviceKeys()
	if err != nil {
		t.Fatalf("EnsureDeviceKeys(second) error = %v, want nil", err)
	}

	if !bytes.Equal(first.Ed25519Seed, second.Ed25519Seed) {
		t.Fatalf("EnsureDeviceKeys() ed25519 seed changed")
	}
	if !bytes.Equal(first.X25519PrivateKey, second.X25519PrivateKey) {
		t.Fatalf("EnsureDeviceKeys() x25519 private key changed")
	}

	peerID, err := first.PeerID()
	if err != nil {
		t.Fatalf("DeviceKeys.PeerID() error = %v, want nil", err)
	}
	if len(peerID) != wire.CanonicalIDLen {
		t.Fatalf("DeviceKeys.PeerID() length = %d, want %d", len(peerID), wire.CanonicalIDLen)
	}
}

func TestPersistJoinedBootstrapUsesCanonicalNetworkIDAndRoundTrips(t *testing.T) {
	t.Parallel()

	store := mustOpenStore(t)
	fixture := mustJoinedBootstrapFixture(t)
	lowerNetworkID := strings.ToLower(fixture.NetworkID)

	if err := store.PersistJoinedBootstrap(fixture.withNetworkID(lowerNetworkID)); err != nil {
		t.Fatalf("PersistJoinedBootstrap(lower network_id) error = %v, want nil", err)
	}

	assertDirExists(t, filepath.Join(store.root, "networks", fixture.NetworkID))
	assertPathMissing(t, filepath.Join(store.root, "networks", lowerNetworkID))

	memberCredential, err := store.LoadSelfMemberCredential(lowerNetworkID)
	if err != nil {
		t.Fatalf("LoadSelfMemberCredential(%q) error = %v, want nil", lowerNetworkID, err)
	}
	if !bytes.Equal(memberCredential, fixture.SelfMemberCredential) {
		t.Fatalf("LoadSelfMemberCredential(%q) mismatch", lowerNetworkID)
	}

	mailboxSecret, err := store.LoadMailboxSecret(lowerNetworkID)
	if err != nil {
		t.Fatalf("LoadMailboxSecret(%q) error = %v, want nil", lowerNetworkID, err)
	}
	if !bytes.Equal(mailboxSecret, fixture.MailboxSecret) {
		t.Fatalf("LoadMailboxSecret(%q) mismatch", lowerNetworkID)
	}

	broker, err := store.LoadRuntimeBroker(lowerNetworkID)
	if err != nil {
		t.Fatalf("LoadRuntimeBroker(%q) error = %v, want nil", lowerNetworkID, err)
	}
	if broker.Endpoint != fixture.RuntimeBroker.Endpoint {
		t.Fatalf("LoadRuntimeBroker(%q) endpoint = %q, want %q", lowerNetworkID, broker.Endpoint, fixture.RuntimeBroker.Endpoint)
	}
	if strings.Join(broker.StunServers, ",") != strings.Join(fixture.RuntimeBroker.StunServers, ",") {
		t.Fatalf("LoadRuntimeBroker(%q) stun_servers = %#v, want %#v", lowerNetworkID, broker.StunServers, fixture.RuntimeBroker.StunServers)
	}

	roster, err := store.LoadRosterSnapshot(lowerNetworkID)
	if err != nil {
		t.Fatalf("LoadRosterSnapshot(%q) error = %v, want nil", lowerNetworkID, err)
	}
	if diff := diffRosterSnapshot(fixture.RosterSnapshot, roster); diff != "" {
		t.Fatalf("LoadRosterSnapshot(%q) mismatch: %s", lowerNetworkID, diff)
	}
}

func TestReplaceRosterSnapshotKeepsPreviousFileOnFailure(t *testing.T) {
	t.Parallel()

	store := mustOpenStore(t)
	fixture := mustJoinedBootstrapFixture(t)
	if err := store.PersistJoinedBootstrap(fixture); err != nil {
		t.Fatalf("PersistJoinedBootstrap() error = %v, want nil", err)
	}

	originalData, err := os.ReadFile(filepath.Join(store.root, "networks", fixture.NetworkID, "roster_snapshot.json"))
	if err != nil {
		t.Fatalf("ReadFile(roster_snapshot.json) error = %v, want nil", err)
	}

	realWriteFile := store.ops.writeFile
	store.ops.writeFile = func(path string, data []byte, perm os.FileMode) error {
		if filepath.Base(path) != "roster_snapshot.json" {
			return realWriteFile(path, data, perm)
		}

		tmp, err := os.CreateTemp(filepath.Dir(path), ".rewrite-*")
		if err != nil {
			return err
		}
		if _, err := tmp.Write(data); err != nil {
			_ = tmp.Close()
			return err
		}
		if err := tmp.Close(); err != nil {
			return err
		}
		return errors.New("forced rewrite failure before rename")
	}

	replacement := RosterSnapshot{
		Entries: []RosterEntry{
			{
				PeerID:           fixture.RosterSnapshot.Entries[0].PeerID,
				MemberCredential: []byte("replacement"),
				DeviceName:       "updated",
				Platform:         "linux",
			},
		},
	}
	err = store.ReplaceRosterSnapshot(fixture.NetworkID, replacement)
	if err == nil || !strings.Contains(err.Error(), "forced rewrite failure before rename") {
		t.Fatalf("ReplaceRosterSnapshot() error = %v, want forced rewrite failure", err)
	}

	gotData, err := os.ReadFile(filepath.Join(store.root, "networks", fixture.NetworkID, "roster_snapshot.json"))
	if err != nil {
		t.Fatalf("ReadFile(roster_snapshot.json after failed rewrite) error = %v, want nil", err)
	}
	if !bytes.Equal(gotData, originalData) {
		t.Fatalf("ReplaceRosterSnapshot() rewrote roster file on failure")
	}
}

func TestPersistJoinedBootstrapFailureLeavesNoVisiblePartialNetwork(t *testing.T) {
	t.Parallel()

	store := mustOpenStore(t)
	fixture := mustJoinedBootstrapFixture(t)

	writeCount := 0
	realWriteFile := store.ops.writeFile
	store.ops.writeFile = func(path string, data []byte, perm os.FileMode) error {
		writeCount++
		if writeCount == 3 {
			return errors.New("forced bootstrap write failure")
		}
		return realWriteFile(path, data, perm)
	}

	err := store.PersistJoinedBootstrap(fixture)
	if err == nil || !strings.Contains(err.Error(), "forced bootstrap write failure") {
		t.Fatalf("PersistJoinedBootstrap() error = %v, want forced bootstrap write failure", err)
	}

	assertPathMissing(t, filepath.Join(store.root, "networks", fixture.NetworkID))
	stageEntries, readErr := os.ReadDir(filepath.Join(store.root, "networks"))
	if readErr != nil {
		t.Fatalf("ReadDir(networks) error = %v, want nil", readErr)
	}
	for _, entry := range stageEntries {
		if strings.Contains(entry.Name(), fixture.NetworkID) {
			t.Fatalf("PersistJoinedBootstrap() left staged network entry %q", entry.Name())
		}
	}

	store.ops.writeFile = realWriteFile
	if err := store.PersistJoinedBootstrap(fixture); err != nil {
		t.Fatalf("PersistJoinedBootstrap(retry) error = %v, want nil", err)
	}
}

func TestEnrollHandledRequestRoundTripWithoutJoinedNetwork(t *testing.T) {
	t.Parallel()

	store := mustOpenStore(t)
	fixture := mustJoinedBootstrapFixture(t)
	record := EnrollHandledRequest{
		MsgID:              mustMsgID(t, "MFRGGZDFMZTWQ2LKNNWG23TPOI"),
		RequestFingerprint: []byte("fingerprint"),
		ResponseCiphertext: []byte("ciphertext"),
	}

	if err := store.StoreEnrollHandledRequest(fixture.NetworkID, record); err != nil {
		t.Fatalf("StoreEnrollHandledRequest() error = %v, want nil", err)
	}

	loaded, err := store.LoadEnrollHandledRequest(fixture.NetworkID, record.MsgID)
	if err != nil {
		t.Fatalf("LoadEnrollHandledRequest() error = %v, want nil", err)
	}
	if loaded.MsgID != record.MsgID {
		t.Fatalf("LoadEnrollHandledRequest().MsgID = %q, want %q", loaded.MsgID, record.MsgID)
	}
	if !bytes.Equal(loaded.RequestFingerprint, record.RequestFingerprint) {
		t.Fatalf("LoadEnrollHandledRequest().RequestFingerprint mismatch")
	}
	if !bytes.Equal(loaded.ResponseCiphertext, record.ResponseCiphertext) {
		t.Fatalf("LoadEnrollHandledRequest().ResponseCiphertext mismatch")
	}
}

func TestEnrollHandledRequestFailureLeavesNoPartialRecord(t *testing.T) {
	t.Parallel()

	store := mustOpenStore(t)
	fixture := mustJoinedBootstrapFixture(t)
	record := EnrollHandledRequest{
		MsgID:              mustMsgID(t, "MFRGGZDFMZTWQ2LKNNWG23TPOI"),
		RequestFingerprint: []byte("fingerprint"),
		ResponseCiphertext: []byte("ciphertext"),
	}

	realWriteFile := store.ops.writeFile
	store.ops.writeFile = func(path string, data []byte, perm os.FileMode) error {
		if filepath.Base(path) != record.MsgID+".json" {
			return realWriteFile(path, data, perm)
		}
		return errors.New("forced enroll handled write failure")
	}

	err := store.StoreEnrollHandledRequest(fixture.NetworkID, record)
	if err == nil || !strings.Contains(err.Error(), "forced enroll handled write failure") {
		t.Fatalf("StoreEnrollHandledRequest() error = %v, want forced enroll handled write failure", err)
	}

	cachePath := filepath.Join(store.root, "device", "enroll_handled", fixture.NetworkID, record.MsgID+".json")
	assertPathMissing(t, cachePath)
}

func TestEnrollHandledRequestDoesNotExposeJoinedNetwork(t *testing.T) {
	t.Parallel()

	store := mustOpenStore(t)
	fixture := mustJoinedBootstrapFixture(t)
	record := EnrollHandledRequest{
		MsgID:              mustMsgID(t, "MFRGGZDFMZTWQ2LKNNWG23TPOI"),
		RequestFingerprint: []byte("fingerprint"),
		ResponseCiphertext: []byte("ciphertext"),
	}

	if err := store.StoreEnrollHandledRequest(fixture.NetworkID, record); err != nil {
		t.Fatalf("StoreEnrollHandledRequest() error = %v, want nil", err)
	}
	if _, err := store.LoadSelfMemberCredential(fixture.NetworkID); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("LoadSelfMemberCredential() error = %v, want %v", err, os.ErrNotExist)
	}
}

func TestReloadAndTopicScopeRemainStableAcrossRestart(t *testing.T) {
	t.Parallel()

	root := filepath.Join(t.TempDir(), "state")
	store, err := Open(root)
	if err != nil {
		t.Fatalf("Open(%q) error = %v, want nil", root, err)
	}

	fixture := mustJoinedBootstrapFixture(t)
	if err := store.PersistJoinedBootstrap(fixture); err != nil {
		t.Fatalf("PersistJoinedBootstrap() error = %v, want nil", err)
	}

	reopened, err := Open(root)
	if err != nil {
		t.Fatalf("Open(reopen %q) error = %v, want nil", root, err)
	}

	scopeA, err := store.LoadTopicScope(fixture.NetworkID)
	if err != nil {
		t.Fatalf("LoadTopicScope(first) error = %v, want nil", err)
	}
	scopeB, err := reopened.LoadTopicScope(fixture.NetworkID)
	if err != nil {
		t.Fatalf("LoadTopicScope(reopen) error = %v, want nil", err)
	}
	if scopeA.NetRoot != scopeB.NetRoot {
		t.Fatalf("LoadTopicScope().NetRoot = %q, want %q", scopeB.NetRoot, scopeA.NetRoot)
	}

	peerID := fixture.RosterSnapshot.Entries[0].PeerID
	presenceA, err := scopeA.PresenceTopic(strings.ToLower(peerID))
	if err != nil {
		t.Fatalf("PresenceTopic(%q) error = %v, want nil", strings.ToLower(peerID), err)
	}
	presenceB, err := scopeB.PresenceTopic(peerID)
	if err != nil {
		t.Fatalf("PresenceTopic(%q) reopen error = %v, want nil", peerID, err)
	}
	if presenceA != presenceB {
		t.Fatalf("PresenceTopic(%q) = %q, want %q", peerID, presenceB, presenceA)
	}

	inboxA, err := scopeA.InboxTopic(peerID)
	if err != nil {
		t.Fatalf("InboxTopic(%q) error = %v, want nil", peerID, err)
	}
	inboxB, err := scopeB.InboxTopic(strings.ToLower(peerID))
	if err != nil {
		t.Fatalf("InboxTopic(%q) reopen error = %v, want nil", strings.ToLower(peerID), err)
	}
	if inboxA != inboxB {
		t.Fatalf("InboxTopic(%q) = %q, want %q", peerID, inboxB, inboxA)
	}
}

func TestPermissionDriftIsCorrectedOnTouchedPaths(t *testing.T) {
	t.Parallel()

	if runtime.GOOS == "windows" {
		t.Skip("POSIX permission correction is not asserted on Windows")
	}

	store := mustOpenStore(t)
	fixture := mustJoinedBootstrapFixture(t)
	keys, err := store.EnsureDeviceKeys()
	if err != nil {
		t.Fatalf("EnsureDeviceKeys() error = %v, want nil", err)
	}
	record := EnrollHandledRequest{
		MsgID:              mustMsgID(t, "MFRGGZDFMZTWQ2LKNNWG23TPOI"),
		RequestFingerprint: []byte("fingerprint"),
		ResponseCiphertext: []byte("ciphertext"),
	}
	if err := store.StoreEnrollHandledRequest(fixture.NetworkID, record); err != nil {
		t.Fatalf("StoreEnrollHandledRequest() error = %v, want nil", err)
	}
	if err := store.PersistJoinedBootstrap(fixture); err != nil {
		t.Fatalf("PersistJoinedBootstrap() error = %v, want nil", err)
	}

	paths := []string{
		store.root,
		filepath.Join(store.root, "device"),
		filepath.Join(store.root, "device", "ed25519.key"),
		filepath.Join(store.root, "device", "x25519.key"),
		filepath.Join(store.root, "device", "enroll_handled"),
		filepath.Join(store.root, "device", "enroll_handled", fixture.NetworkID),
		filepath.Join(store.root, "device", "enroll_handled", fixture.NetworkID, record.MsgID+".json"),
		filepath.Join(store.root, "networks", fixture.NetworkID),
		filepath.Join(store.root, "networks", fixture.NetworkID, "member_credential.bin"),
		filepath.Join(store.root, "networks", fixture.NetworkID, "mailbox_secret.bin"),
		filepath.Join(store.root, "networks", fixture.NetworkID, "broker.json"),
		filepath.Join(store.root, "networks", fixture.NetworkID, "roster_snapshot.json"),
	}
	for _, path := range paths {
		if err := os.Chmod(path, 0o777); err != nil {
			t.Fatalf("Chmod(%q, 0777) error = %v, want nil", path, err)
		}
	}

	reloadedKeys, err := store.LoadDeviceKeys()
	if err != nil {
		t.Fatalf("LoadDeviceKeys() error = %v, want nil", err)
	}
	if !bytes.Equal(reloadedKeys.Ed25519Seed, keys.Ed25519Seed) {
		t.Fatalf("LoadDeviceKeys() ed25519 seed mismatch after permission repair")
	}
	if _, err := store.LoadRuntimeBroker(fixture.NetworkID); err != nil {
		t.Fatalf("LoadRuntimeBroker() error = %v, want nil", err)
	}
	if _, err := store.LoadRosterSnapshot(fixture.NetworkID); err != nil {
		t.Fatalf("LoadRosterSnapshot() error = %v, want nil", err)
	}
	if _, err := store.LoadEnrollHandledRequest(fixture.NetworkID, record.MsgID); err != nil {
		t.Fatalf("LoadEnrollHandledRequest() error = %v, want nil", err)
	}

	assertMode(t, store.root, dirPerm)
	assertMode(t, filepath.Join(store.root, "device"), dirPerm)
	assertMode(t, filepath.Join(store.root, "device", "ed25519.key"), filePerm)
	assertMode(t, filepath.Join(store.root, "device", "x25519.key"), filePerm)
	assertMode(t, filepath.Join(store.root, "device", "enroll_handled"), dirPerm)
	assertMode(t, filepath.Join(store.root, "device", "enroll_handled", fixture.NetworkID), dirPerm)
	assertMode(t, filepath.Join(store.root, "device", "enroll_handled", fixture.NetworkID, record.MsgID+".json"), filePerm)
	assertMode(t, filepath.Join(store.root, "networks", fixture.NetworkID), dirPerm)
	assertMode(t, filepath.Join(store.root, "networks", fixture.NetworkID, "member_credential.bin"), filePerm)
	assertMode(t, filepath.Join(store.root, "networks", fixture.NetworkID, "mailbox_secret.bin"), filePerm)
	assertMode(t, filepath.Join(store.root, "networks", fixture.NetworkID, "broker.json"), filePerm)
	assertMode(t, filepath.Join(store.root, "networks", fixture.NetworkID, "roster_snapshot.json"), filePerm)
}

func TestContractForFuture02And04Callers(t *testing.T) {
	t.Parallel()

	store := mustOpenStore(t)
	fixture := mustJoinedBootstrapFixture(t)
	if err := store.PersistJoinedBootstrap(fixture); err != nil {
		t.Fatalf("PersistJoinedBootstrap() error = %v, want nil", err)
	}

	loadedRoster, err := store.LoadRosterSnapshot(fixture.NetworkID)
	if err != nil {
		t.Fatalf("LoadRosterSnapshot(%q) error = %v, want nil", fixture.NetworkID, err)
	}
	if diff := diffRosterSnapshot(fixture.RosterSnapshot, loadedRoster); diff != "" {
		t.Fatalf("LoadRosterSnapshot(%q) mismatch: %s", fixture.NetworkID, diff)
	}

	scope, err := store.LoadTopicScope(fixture.NetworkID)
	if err != nil {
		t.Fatalf("LoadTopicScope(%q) error = %v, want nil", fixture.NetworkID, err)
	}

	remote := loadedRoster.Entries[1]
	presenceTopic, err := scope.PresenceTopic(remote.PeerID)
	if err != nil {
		t.Fatalf("PresenceTopic(%q) error = %v, want nil", remote.PeerID, err)
	}
	inboxTopic, err := scope.InboxTopic(strings.ToLower(remote.PeerID))
	if err != nil {
		t.Fatalf("InboxTopic(%q) error = %v, want nil", strings.ToLower(remote.PeerID), err)
	}

	if !strings.HasPrefix(presenceTopic, "mp/v1/net/"+scope.NetRoot+"/presence/") {
		t.Fatalf("PresenceTopic(%q) = %q, want prefix %q", remote.PeerID, presenceTopic, "mp/v1/net/"+scope.NetRoot+"/presence/")
	}
	if !strings.HasPrefix(inboxTopic, "mp/v1/net/"+scope.NetRoot+"/inbox/") {
		t.Fatalf("InboxTopic(%q) = %q, want prefix %q", remote.PeerID, inboxTopic, "mp/v1/net/"+scope.NetRoot+"/inbox/")
	}
}

func mustOpenStore(t *testing.T) *Store {
	t.Helper()

	store, err := Open(filepath.Join(t.TempDir(), "state"))
	if err != nil {
		t.Fatalf("Open(temp state) error = %v, want nil", err)
	}
	return store
}

func mustJoinedBootstrapFixture(t *testing.T) JoinedBootstrap {
	t.Helper()

	networkID, err := wire.EncodeNetworkID(bytes.Repeat([]byte{0x5a}, wire.RawIDLen))
	if err != nil {
		t.Fatalf("EncodeNetworkID() error = %v, want nil", err)
	}

	peerA := mustPeerID(t, 0x11)
	peerB := mustPeerID(t, 0x22)

	return JoinedBootstrap{
		NetworkID:            networkID,
		SelfMemberCredential: []byte("self-member-credential"),
		MailboxSecret:        bytes.Repeat([]byte{0x33}, mailboxSecretSize),
		RuntimeBroker: RuntimeBroker{
			Endpoint:    "broker.example.net:1883",
			StunServers: []string{"stun1.example.net:3478", "stun2.example.net:3478"},
		},
		RosterSnapshot: RosterSnapshot{
			Entries: []RosterEntry{
				{
					PeerID:           peerA,
					MemberCredential: []byte("credential-a"),
					DeviceName:       "alpha",
					Platform:         "linux",
				},
				{
					PeerID:           peerB,
					MemberCredential: []byte("credential-b"),
					DeviceName:       "beta",
					Platform:         "windows",
				},
			},
		},
	}
}

func (j JoinedBootstrap) withNetworkID(networkID string) JoinedBootstrap {
	j.NetworkID = networkID
	return j
}

func mustPeerID(t *testing.T, seedByte byte) string {
	t.Helper()

	pub := bytes.Repeat([]byte{seedByte}, 32)
	peerID, err := wire.PeerIDFromEd25519Pub(pub)
	if err != nil {
		t.Fatalf("PeerIDFromEd25519Pub(%x) error = %v, want nil", pub, err)
	}
	return peerID
}

func mustMsgID(t *testing.T, value string) string {
	t.Helper()

	msgID, err := wire.CanonicalizeMsgID(value)
	if err != nil {
		t.Fatalf("CanonicalizeMsgID(%q) error = %v, want nil", value, err)
	}
	return msgID
}

func diffRosterSnapshot(want, got RosterSnapshot) string {
	if len(want.Entries) != len(got.Entries) {
		return "entry count mismatch"
	}
	for i := range want.Entries {
		switch {
		case want.Entries[i].PeerID != got.Entries[i].PeerID:
			return "peer_id mismatch"
		case !bytes.Equal(want.Entries[i].MemberCredential, got.Entries[i].MemberCredential):
			return "member credential mismatch"
		case want.Entries[i].DeviceName != got.Entries[i].DeviceName:
			return "device_name mismatch"
		case want.Entries[i].Platform != got.Entries[i].Platform:
			return "platform mismatch"
		}
	}
	return ""
}

func assertDirExists(t *testing.T, path string) {
	t.Helper()

	info, err := os.Stat(path)
	if err != nil {
		t.Fatalf("Stat(%q) error = %v, want existing directory", path, err)
	}
	if !info.IsDir() {
		t.Fatalf("Stat(%q) mode = %v, want directory", path, info.Mode())
	}
}

func assertPathMissing(t *testing.T, path string) {
	t.Helper()

	if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("Stat(%q) error = %v, want %v", path, err, os.ErrNotExist)
	}
}

func assertMode(t *testing.T, path string, want os.FileMode) {
	t.Helper()

	if runtime.GOOS == "windows" {
		return
	}

	info, err := os.Stat(path)
	if err != nil {
		t.Fatalf("Stat(%q) error = %v, want nil", path, err)
	}
	if got := info.Mode().Perm(); got != want {
		t.Fatalf("Stat(%q) mode = %#o, want %#o", path, got, want)
	}
}
