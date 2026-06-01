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
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/miopunch/miopunch/internal/atomicfile"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

const (
	dirPerm  = 0o700
	filePerm = 0o600
)

var errIncompleteState = errors.New("incomplete persisted state")

type fileOps struct {
	chmod     func(string, os.FileMode) error
	mkdirAll  func(string, os.FileMode) error
	mkdirTemp func(string, string) (string, error)
	readFile  func(string) ([]byte, error)
	removeAll func(string) error
	rename    func(string, string) error
	stat      func(string) (os.FileInfo, error)
	writeFile func(string, []byte, os.FileMode) error
}

func defaultFileOps() fileOps {
	return fileOps{
		chmod:     os.Chmod,
		mkdirAll:  os.MkdirAll,
		mkdirTemp: os.MkdirTemp,
		readFile:  os.ReadFile,
		removeAll: os.RemoveAll,
		rename:    os.Rename,
		stat:      os.Stat,
		writeFile: atomicfile.WriteFile,
	}
}

type devicePaths struct {
	dir               string
	ed25519Key        string
	x25519Key         string
	enrollHandledRoot string
}

type networkPaths struct {
	id               string
	dir              string
	memberCredential string
	mailboxSecret    string
	broker           string
	rosterSnapshot   string
}

type enrollHandledPaths struct {
	networkID string
	dir       string
	file      string
}

// Store owns the current v1 persistence root and typed state APIs below it.
type Store struct {
	root string
	ops  fileOps
}

// Open opens current v1 persistence against a caller-supplied root directory.
func Open(root string) (*Store, error) {
	trimmed := strings.TrimSpace(root)
	if trimmed == "" {
		return nil, errors.New("empty persistence root")
	}

	store := &Store{
		root: filepath.Clean(trimmed),
		ops:  defaultFileOps(),
	}
	if err := store.ensureDir(store.root); err != nil {
		return nil, fmt.Errorf("ensure root: %w", err)
	}
	if err := store.ensureDir(store.devicePaths().dir); err != nil {
		return nil, fmt.Errorf("ensure device dir: %w", err)
	}
	return store, nil
}

// EnsureDeviceKeys loads the persisted device keys or creates them on first use.
func (s *Store) EnsureDeviceKeys() (DeviceKeys, error) {
	if err := s.repairDir(s.root); err != nil {
		return DeviceKeys{}, fmt.Errorf("repair root dir: %w", err)
	}

	keys, err := s.LoadDeviceKeys()
	if err == nil {
		return keys, nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return DeviceKeys{}, err
	}

	keys, err = newDeviceKeys()
	if err != nil {
		return DeviceKeys{}, err
	}
	if err := s.ensureDir(s.devicePaths().dir); err != nil {
		return DeviceKeys{}, fmt.Errorf("ensure device dir: %w", err)
	}
	if err := s.writeFile(s.devicePaths().ed25519Key, keys.Ed25519Seed); err != nil {
		return DeviceKeys{}, fmt.Errorf("write ed25519 key: %w", err)
	}
	if err := s.writeFile(s.devicePaths().x25519Key, keys.X25519PrivateKey); err != nil {
		return DeviceKeys{}, fmt.Errorf("write x25519 key: %w", err)
	}
	return keys, nil
}

// LoadDeviceKeys loads the persisted device-global key material.
func (s *Store) LoadDeviceKeys() (DeviceKeys, error) {
	paths := s.devicePaths()
	state, err := s.deviceKeyState(paths)
	if err != nil {
		return DeviceKeys{}, err
	}
	if !state.complete {
		return DeviceKeys{}, os.ErrNotExist
	}

	if err := s.repairDir(paths.dir); err != nil {
		return DeviceKeys{}, fmt.Errorf("repair device dir: %w", err)
	}

	edSeed, err := s.readFile(paths.ed25519Key)
	if err != nil {
		return DeviceKeys{}, fmt.Errorf("read ed25519 key: %w", err)
	}
	xPriv, err := s.readFile(paths.x25519Key)
	if err != nil {
		return DeviceKeys{}, fmt.Errorf("read x25519 key: %w", err)
	}

	keys := DeviceKeys{
		Ed25519Seed:      edSeed,
		X25519PrivateKey: xPriv,
	}
	if err := validateDeviceKeys(keys); err != nil {
		return DeviceKeys{}, err
	}
	return keys, nil
}

// LoadSelfMemberCredential loads the joined network's self member credential.
func (s *Store) LoadSelfMemberCredential(networkID string) ([]byte, error) {
	paths, err := s.resolveNetworkPaths(networkID)
	if err != nil {
		return nil, err
	}
	if err := s.requireCompleteJoinedNetwork(paths); err != nil {
		return nil, err
	}
	data, err := s.readFile(paths.memberCredential)
	if err != nil {
		return nil, fmt.Errorf("read self member credential: %w", err)
	}
	if len(data) == 0 {
		return nil, fmt.Errorf("self member credential is empty")
	}
	return data, nil
}

// LoadMailboxSecret loads the joined network's mailbox secret.
func (s *Store) LoadMailboxSecret(networkID string) ([]byte, error) {
	paths, err := s.resolveNetworkPaths(networkID)
	if err != nil {
		return nil, err
	}
	if err := s.requireCompleteJoinedNetwork(paths); err != nil {
		return nil, err
	}
	data, err := s.readFile(paths.mailboxSecret)
	if err != nil {
		return nil, fmt.Errorf("read mailbox secret: %w", err)
	}
	if len(data) != mailboxSecretSize {
		return nil, fmt.Errorf("invalid mailbox secret length: %d", len(data))
	}
	return data, nil
}

// LoadRuntimeBroker loads the joined network's single runtime broker endpoint.
func (s *Store) LoadRuntimeBroker(networkID string) (RuntimeBroker, error) {
	paths, err := s.resolveNetworkPaths(networkID)
	if err != nil {
		return RuntimeBroker{}, err
	}
	if err := s.requireCompleteJoinedNetwork(paths); err != nil {
		return RuntimeBroker{}, err
	}
	data, err := s.readFile(paths.broker)
	if err != nil {
		return RuntimeBroker{}, fmt.Errorf("read runtime broker: %w", err)
	}

	var record brokerRecord
	if err := json.Unmarshal(data, &record); err != nil {
		return RuntimeBroker{}, fmt.Errorf("unmarshal runtime broker: %w", err)
	}
	return normalizeBroker(RuntimeBroker{Endpoint: record.Endpoint})
}

// LoadRosterSnapshot loads the whole trusted roster snapshot for networkID.
func (s *Store) LoadRosterSnapshot(networkID string) (RosterSnapshot, error) {
	paths, err := s.resolveNetworkPaths(networkID)
	if err != nil {
		return RosterSnapshot{}, err
	}
	if err := s.requireCompleteJoinedNetwork(paths); err != nil {
		return RosterSnapshot{}, err
	}
	data, err := s.readFile(paths.rosterSnapshot)
	if err != nil {
		return RosterSnapshot{}, fmt.Errorf("read roster snapshot: %w", err)
	}

	var records []rosterEntryRecord
	if err := json.Unmarshal(data, &records); err != nil {
		return RosterSnapshot{}, fmt.Errorf("unmarshal roster snapshot: %w", err)
	}

	entries := make([]RosterEntry, 0, len(records))
	for i, record := range records {
		memberCredential, err := base64.RawURLEncoding.DecodeString(record.MemberCredential)
		if err != nil {
			return RosterSnapshot{}, fmt.Errorf("decode roster entry %d member credential: %w", i, err)
		}
		entries = append(entries, RosterEntry{
			PeerID:           record.PeerID,
			MemberCredential: memberCredential,
			DeviceName:       record.DeviceName,
			Platform:         record.Platform,
		})
	}
	return normalizeSnapshot(RosterSnapshot{Entries: entries})
}

// ReplaceRosterSnapshot atomically replaces the whole trusted roster snapshot.
func (s *Store) ReplaceRosterSnapshot(networkID string, snapshot RosterSnapshot) error {
	paths, err := s.resolveNetworkPaths(networkID)
	if err != nil {
		return err
	}
	if err := s.requireCompleteJoinedNetwork(paths); err != nil {
		return err
	}

	normalizedSnapshot, err := normalizeSnapshot(snapshot)
	if err != nil {
		return err
	}
	data, err := marshalRosterSnapshot(normalizedSnapshot)
	if err != nil {
		return err
	}
	if err := s.writeFile(paths.rosterSnapshot, data); err != nil {
		return fmt.Errorf("write roster snapshot: %w", err)
	}
	return nil
}

// LoadEnrollHandledRequest loads one authority-side enroll replay record.
func (s *Store) LoadEnrollHandledRequest(networkID string, msgID string) (EnrollHandledRequest, error) {
	paths, err := s.resolveEnrollHandledPaths(networkID, msgID)
	if err != nil {
		return EnrollHandledRequest{}, err
	}
	if err := s.repairDir(s.devicePaths().enrollHandledRoot); err != nil {
		return EnrollHandledRequest{}, err
	}
	if err := s.repairDir(paths.dir); err != nil {
		return EnrollHandledRequest{}, err
	}

	data, err := s.readFile(paths.file)
	if err != nil {
		return EnrollHandledRequest{}, err
	}

	var record enrollHandledRecord
	if err := json.Unmarshal(data, &record); err != nil {
		return EnrollHandledRequest{}, fmt.Errorf("unmarshal enroll handled request: %w", err)
	}

	requestFingerprint, err := base64.RawURLEncoding.DecodeString(record.RequestFingerprint)
	if err != nil {
		return EnrollHandledRequest{}, fmt.Errorf("decode request fingerprint: %w", err)
	}
	responseCiphertext, err := base64.RawURLEncoding.DecodeString(record.ResponseCiphertext)
	if err != nil {
		return EnrollHandledRequest{}, fmt.Errorf("decode response ciphertext: %w", err)
	}

	return normalizeEnrollHandledRequest(EnrollHandledRequest{
		MsgID:              record.MsgID,
		RequestFingerprint: requestFingerprint,
		ResponseCiphertext: responseCiphertext,
	})
}

// StoreEnrollHandledRequest atomically stores one authority-side enroll replay
// record under the device-scoped authority replay-cache directory.
func (s *Store) StoreEnrollHandledRequest(networkID string, record EnrollHandledRequest) error {
	normalizedRecord, err := normalizeEnrollHandledRequest(record)
	if err != nil {
		return err
	}

	paths, err := s.resolveEnrollHandledPaths(networkID, normalizedRecord.MsgID)
	if err != nil {
		return err
	}
	if err := s.ensureDir(paths.dir); err != nil {
		return fmt.Errorf("ensure enroll handled dir: %w", err)
	}
	if err := s.repairDir(s.devicePaths().enrollHandledRoot); err != nil {
		return fmt.Errorf("repair enroll handled root: %w", err)
	}
	if err := s.repairDir(paths.dir); err != nil {
		return fmt.Errorf("repair enroll handled dir: %w", err)
	}

	data, err := marshalEnrollHandledRequest(normalizedRecord)
	if err != nil {
		return err
	}
	if err := s.writeFile(paths.file, data); err != nil {
		return fmt.Errorf("write enroll handled request: %w", err)
	}
	return nil
}

// PersistJoinedBootstrap atomically makes joined bootstrap state visible for one
// network or leaves that network absent after failure.
func (s *Store) PersistJoinedBootstrap(joined JoinedBootstrap) error {
	normalizedJoined, err := normalizeJoinedBootstrap(joined)
	if err != nil {
		return err
	}
	paths, err := s.resolveNetworkPaths(normalizedJoined.NetworkID)
	if err != nil {
		return err
	}

	if err := s.ensureDir(s.networksDir()); err != nil {
		return fmt.Errorf("ensure networks dir: %w", err)
	}

	complete, err := s.joinedNetworkComplete(paths)
	switch {
	case err == nil && complete:
		return nil
	case err != nil && errors.Is(err, errIncompleteState):
		return fmt.Errorf("joined network %s is incomplete: %w", paths.id, err)
	case err != nil && !errors.Is(err, os.ErrNotExist):
		return fmt.Errorf("check joined network %s: %w", paths.id, err)
	}

	stageDir, err := s.ops.mkdirTemp(s.networksDir(), "."+paths.id+".stage-")
	if err != nil {
		return fmt.Errorf("create staged network dir: %w", err)
	}

	cleanupStage := true
	defer func() {
		if cleanupStage {
			_ = s.ops.removeAll(stageDir)
		}
	}()

	if err := s.ensureDir(stageDir); err != nil {
		return fmt.Errorf("ensure staged network dir: %w", err)
	}
	if err := s.writeBootstrapFiles(stageDir, normalizedJoined); err != nil {
		return err
	}
	if err := s.renameStageDir(stageDir, paths); err != nil {
		return err
	}

	cleanupStage = false
	return nil
}

// LoadTopicScope loads the joined network mailbox secret and derives its topic
// scope.
func (s *Store) LoadTopicScope(networkID string) (TopicScope, error) {
	canonicalNetworkID, err := wire.CanonicalizeNetworkID(networkID)
	if err != nil {
		return TopicScope{}, fmt.Errorf("canonicalize network_id: %w", err)
	}
	mailboxSecret, err := s.LoadMailboxSecret(canonicalNetworkID)
	if err != nil {
		return TopicScope{}, err
	}
	return newTopicScope(canonicalNetworkID, mailboxSecret)
}

type deviceKeyState struct {
	complete bool
}

type brokerRecord struct {
	Endpoint string `json:"endpoint"`
}

type rosterEntryRecord struct {
	PeerID           string `json:"peer_id"`
	MemberCredential string `json:"member_credential"`
	DeviceName       string `json:"device_name,omitempty"`
	Platform         string `json:"platform,omitempty"`
}

type enrollHandledRecord struct {
	MsgID              string `json:"msg_id"`
	RequestFingerprint string `json:"request_fingerprint"`
	ResponseCiphertext string `json:"response_ciphertext"`
}

func newDeviceKeys() (DeviceKeys, error) {
	edSeed := make([]byte, deviceKeySize)
	if _, err := rand.Read(edSeed); err != nil {
		return DeviceKeys{}, fmt.Errorf("read ed25519 seed: %w", err)
	}

	xPriv := make([]byte, deviceKeySize)
	if _, err := rand.Read(xPriv); err != nil {
		return DeviceKeys{}, fmt.Errorf("read x25519 private key: %w", err)
	}

	return DeviceKeys{
		Ed25519Seed:      edSeed,
		X25519PrivateKey: xPriv,
	}, nil
}

func (s *Store) devicePaths() devicePaths {
	deviceDir := filepath.Join(s.root, "device")
	return devicePaths{
		dir:               deviceDir,
		ed25519Key:        filepath.Join(deviceDir, "ed25519.key"),
		x25519Key:         filepath.Join(deviceDir, "x25519.key"),
		enrollHandledRoot: filepath.Join(deviceDir, "enroll_handled"),
	}
}

func (s *Store) networksDir() string {
	return filepath.Join(s.root, "networks")
}

func (s *Store) resolveNetworkPaths(networkID string) (networkPaths, error) {
	canonicalNetworkID, err := wire.CanonicalizeNetworkID(networkID)
	if err != nil {
		return networkPaths{}, fmt.Errorf("canonicalize network_id: %w", err)
	}

	networkDir := filepath.Join(s.networksDir(), canonicalNetworkID)
	return networkPaths{
		id:               canonicalNetworkID,
		dir:              networkDir,
		memberCredential: filepath.Join(networkDir, "member_credential.bin"),
		mailboxSecret:    filepath.Join(networkDir, "mailbox_secret.bin"),
		broker:           filepath.Join(networkDir, "broker.json"),
		rosterSnapshot:   filepath.Join(networkDir, "roster_snapshot.json"),
	}, nil
}

func (s *Store) resolveEnrollHandledPaths(networkID string, msgID string) (enrollHandledPaths, error) {
	canonicalNetworkID, err := wire.CanonicalizeNetworkID(networkID)
	if err != nil {
		return enrollHandledPaths{}, fmt.Errorf("canonicalize network_id: %w", err)
	}
	canonicalMsgID, err := wire.CanonicalizeMsgID(msgID)
	if err != nil {
		return enrollHandledPaths{}, fmt.Errorf("canonicalize msg_id: %w", err)
	}

	dir := filepath.Join(s.devicePaths().enrollHandledRoot, canonicalNetworkID)
	return enrollHandledPaths{
		networkID: canonicalNetworkID,
		dir:       dir,
		file:      filepath.Join(dir, canonicalMsgID+".json"),
	}, nil
}

func (s *Store) deviceKeyState(paths devicePaths) (deviceKeyState, error) {
	edStat, edErr := s.ops.stat(paths.ed25519Key)
	xStat, xErr := s.ops.stat(paths.x25519Key)

	if errors.Is(edErr, os.ErrNotExist) && errors.Is(xErr, os.ErrNotExist) {
		return deviceKeyState{}, os.ErrNotExist
	}
	if edErr != nil && !errors.Is(edErr, os.ErrNotExist) {
		return deviceKeyState{}, fmt.Errorf("stat ed25519 key: %w", edErr)
	}
	if xErr != nil && !errors.Is(xErr, os.ErrNotExist) {
		return deviceKeyState{}, fmt.Errorf("stat x25519 key: %w", xErr)
	}
	if errors.Is(edErr, os.ErrNotExist) || errors.Is(xErr, os.ErrNotExist) {
		return deviceKeyState{}, errIncompleteState
	}
	if edStat.IsDir() || xStat.IsDir() {
		return deviceKeyState{}, fmt.Errorf("device key path is a directory")
	}
	return deviceKeyState{complete: true}, nil
}

func (s *Store) joinedNetworkComplete(paths networkPaths) (bool, error) {
	dirInfo, err := s.ops.stat(paths.dir)
	if err != nil {
		return false, err
	}
	if !dirInfo.IsDir() {
		return false, fmt.Errorf("network path is not a directory: %s", paths.dir)
	}

	missing := make([]string, 0, 4)
	for _, entry := range []struct {
		name string
		path string
	}{
		{name: "member_credential.bin", path: paths.memberCredential},
		{name: "mailbox_secret.bin", path: paths.mailboxSecret},
		{name: "broker.json", path: paths.broker},
		{name: "roster_snapshot.json", path: paths.rosterSnapshot},
	} {
		info, statErr := s.ops.stat(entry.path)
		if errors.Is(statErr, os.ErrNotExist) {
			missing = append(missing, entry.name)
			continue
		}
		if statErr != nil {
			return false, fmt.Errorf("stat %s: %w", entry.name, statErr)
		}
		if info.IsDir() {
			return false, fmt.Errorf("%s is a directory", entry.name)
		}
	}
	if len(missing) > 0 {
		return false, fmt.Errorf("%w: missing %s", errIncompleteState, strings.Join(missing, ", "))
	}
	return true, nil
}

func (s *Store) requireCompleteJoinedNetwork(paths networkPaths) error {
	complete, err := s.joinedNetworkComplete(paths)
	switch {
	case err == nil && complete:
		if err := s.repairDir(paths.dir); err != nil {
			return fmt.Errorf("repair network dir: %w", err)
		}
		for _, path := range []string{
			paths.memberCredential,
			paths.mailboxSecret,
			paths.broker,
			paths.rosterSnapshot,
		} {
			if err := s.setMode(path, filePerm); err != nil {
				return fmt.Errorf("repair network file mode: %w", err)
			}
		}
		return nil
	case errors.Is(err, os.ErrNotExist):
		return os.ErrNotExist
	default:
		return err
	}
}

func (s *Store) writeBootstrapFiles(stageDir string, joined JoinedBootstrap) error {
	stagePaths := networkPaths{
		id:               joined.NetworkID,
		dir:              stageDir,
		memberCredential: filepath.Join(stageDir, "member_credential.bin"),
		mailboxSecret:    filepath.Join(stageDir, "mailbox_secret.bin"),
		broker:           filepath.Join(stageDir, "broker.json"),
		rosterSnapshot:   filepath.Join(stageDir, "roster_snapshot.json"),
	}

	if err := s.writeFile(stagePaths.memberCredential, joined.SelfMemberCredential); err != nil {
		return fmt.Errorf("write self member credential: %w", err)
	}
	if err := s.writeFile(stagePaths.mailboxSecret, joined.MailboxSecret); err != nil {
		return fmt.Errorf("write mailbox secret: %w", err)
	}

	brokerData, err := marshalBroker(joined.RuntimeBroker)
	if err != nil {
		return err
	}
	if err := s.writeFile(stagePaths.broker, brokerData); err != nil {
		return fmt.Errorf("write runtime broker: %w", err)
	}

	rosterData, err := marshalRosterSnapshot(joined.RosterSnapshot)
	if err != nil {
		return err
	}
	if err := s.writeFile(stagePaths.rosterSnapshot, rosterData); err != nil {
		return fmt.Errorf("write roster snapshot: %w", err)
	}
	return nil
}

func (s *Store) renameStageDir(stageDir string, paths networkPaths) error {
	renameErr := s.ops.rename(stageDir, paths.dir)
	if renameErr == nil {
		return syncDirBestEffort(s.networksDir())
	}

	complete, completeErr := s.joinedNetworkComplete(paths)
	switch {
	case completeErr == nil && complete:
		return nil
	case completeErr != nil && !errors.Is(completeErr, os.ErrNotExist):
		return fmt.Errorf("rename staged network dir: %w", completeErr)
	default:
		return fmt.Errorf("rename staged network dir: %w", renameErr)
	}
}

func marshalBroker(broker RuntimeBroker) ([]byte, error) {
	normalizedBroker, err := normalizeBroker(broker)
	if err != nil {
		return nil, err
	}

	data, err := json.MarshalIndent(brokerRecord{Endpoint: normalizedBroker.Endpoint}, "", "  ")
	if err != nil {
		return nil, fmt.Errorf("marshal runtime broker: %w", err)
	}
	return append(data, '\n'), nil
}

func marshalRosterSnapshot(snapshot RosterSnapshot) ([]byte, error) {
	normalizedSnapshot, err := normalizeSnapshot(snapshot)
	if err != nil {
		return nil, err
	}

	records := make([]rosterEntryRecord, 0, len(normalizedSnapshot.Entries))
	for _, entry := range normalizedSnapshot.Entries {
		records = append(records, rosterEntryRecord{
			PeerID:           entry.PeerID,
			MemberCredential: base64.RawURLEncoding.EncodeToString(entry.MemberCredential),
			DeviceName:       entry.DeviceName,
			Platform:         entry.Platform,
		})
	}

	data, err := json.MarshalIndent(records, "", "  ")
	if err != nil {
		return nil, fmt.Errorf("marshal roster snapshot: %w", err)
	}
	return append(data, '\n'), nil
}

func marshalEnrollHandledRequest(record EnrollHandledRequest) ([]byte, error) {
	normalizedRecord, err := normalizeEnrollHandledRequest(record)
	if err != nil {
		return nil, err
	}

	data, err := json.MarshalIndent(enrollHandledRecord{
		MsgID:              normalizedRecord.MsgID,
		RequestFingerprint: base64.RawURLEncoding.EncodeToString(normalizedRecord.RequestFingerprint),
		ResponseCiphertext: base64.RawURLEncoding.EncodeToString(normalizedRecord.ResponseCiphertext),
	}, "", "  ")
	if err != nil {
		return nil, fmt.Errorf("marshal enroll handled request: %w", err)
	}
	return append(data, '\n'), nil
}

func (s *Store) ensureDir(path string) error {
	if err := s.ops.mkdirAll(path, dirPerm); err != nil {
		return err
	}
	return s.setMode(path, dirPerm)
}

func (s *Store) repairDir(path string) error {
	info, err := s.ops.stat(path)
	if err != nil {
		return err
	}
	if !info.IsDir() {
		return fmt.Errorf("not a directory: %s", path)
	}
	return s.setMode(path, dirPerm)
}

func (s *Store) readFile(path string) ([]byte, error) {
	if err := s.repairDir(s.root); err != nil {
		return nil, err
	}
	if err := s.repairDir(filepath.Dir(path)); err != nil {
		return nil, err
	}
	if err := s.setMode(path, filePerm); err != nil {
		return nil, err
	}

	data, err := s.ops.readFile(path)
	if err != nil {
		return nil, err
	}
	return append([]byte(nil), data...), nil
}

func (s *Store) writeFile(path string, data []byte) error {
	if err := s.repairDir(s.root); err != nil {
		return err
	}
	if err := s.ensureDir(filepath.Dir(path)); err != nil {
		return err
	}
	if err := s.ops.writeFile(path, append([]byte(nil), data...), filePerm); err != nil {
		return err
	}
	return s.setMode(path, filePerm)
}

func (s *Store) setMode(path string, mode os.FileMode) error {
	if runtime.GOOS == "windows" {
		// Go's portable file API does not expose POSIX-equivalent ACL repair on
		// Windows, so current v1 keeps permission tightening as best-effort there.
		return nil
	}
	return s.ops.chmod(path, mode)
}

func syncDirBestEffort(path string) error {
	dir, err := os.Open(path)
	if err != nil {
		return nil
	}
	defer func() {
		_ = dir.Close()
	}()
	_ = dir.Sync()
	return nil
}
