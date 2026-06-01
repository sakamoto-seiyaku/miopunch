package dataplane

import (
	"context"
	"io"
	"testing"
	"time"
)

func TestSessionManagerCloseIfMatchDoesNotCloseSupersedingSession(t *testing.T) {
	t.Parallel()

	key := SessionKey{
		RemotePeerID: "peer-1",
		Protocol:     ProtocolKCP,
		SecurityID:   "peer-1",
		PathFamily:   PathFamilyUDP4,
	}.Normalize()
	manager := NewSessionManager()
	oldSession := &managerTestSession{key: key}
	newSession := &managerTestSession{key: key}

	manager.Put(oldSession)
	manager.Put(newSession)
	manager.CloseIfMatch(oldSession, CloseReasonTransportFatal)

	got, ok := manager.Get(key)
	if !ok {
		t.Fatalf("SessionManager.Get(%+v) ok = false, want true", key)
	}
	if got != newSession {
		t.Fatalf("SessionManager.Get(%+v) = %p, want new session %p", key, got, newSession)
	}
	if !newSession.Healthy() {
		t.Fatalf("new session Healthy() = false, want true")
	}
}

type managerTestSession struct {
	key         SessionKey
	closed      bool
	closeReason CloseReason
}

func (s *managerTestSession) Key() SessionKey {
	return s.key
}

func (s *managerTestSession) OpenStream(context.Context, StreamOpen) (io.ReadWriteCloser, error) {
	return nil, io.ErrClosedPipe
}

func (s *managerTestSession) AcceptStream(context.Context) (*AcceptedStream, error) {
	return nil, io.ErrClosedPipe
}

func (s *managerTestSession) Close(reason CloseReason) error {
	s.closed = true
	s.closeReason = reason
	return nil
}

func (s *managerTestSession) CloseReason() CloseReason {
	return s.closeReason
}

func (s *managerTestSession) Healthy() bool {
	return !s.closed
}

func (s *managerTestSession) LastActivity() time.Time {
	return time.Time{}
}
