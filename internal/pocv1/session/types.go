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
	"context"
	"crypto/ed25519"
	"errors"
	"io"
	"time"

	"github.com/miopunch/miopunch/dataplane"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	"github.com/miopunch/miopunch/internal/pocv1/punch"
	"github.com/miopunch/miopunch/internal/pocv1/wire"
)

// SessionRecipe identifies the current v1 secure session recipe.
type SessionRecipe string

const (
	// SessionRecipeV1KCPYamuxTLS identifies the fixed v1 KCP + TLS 1.3 + yamux recipe.
	SessionRecipeV1KCPYamuxTLS SessionRecipe = "udp+kcp+tls1.3+yamux"
)

// StreamOpen is the logical stream-open envelope carried on each new stream.
type StreamOpen = dataplane.StreamOpen

// AcceptedStream is a logical stream plus its stream-open envelope.
type AcceptedStream = dataplane.AcceptedStream

// PeerSession is the upper-layer contract for a current v1 peer transport session.
type PeerSession = dataplane.PeerSession

// Config carries the inputs required to build a secure session.
type Config struct {
	NetworkID           string
	AuthorityEd25519Pub ed25519.PublicKey
	Store               *persist.Store
	IdleTimeout         time.Duration
}

// Validate returns an error when the configuration is incomplete.
func (c Config) Validate() error {
	if c.Store == nil {
		return errors.New("persistence store is required")
	}
	if len(c.AuthorityEd25519Pub) != ed25519.PublicKeySize {
		return errors.New("authority ed25519 public key has invalid length")
	}
	if _, err := wire.CanonicalizeNetworkID(c.NetworkID); err != nil {
		return err
	}
	return nil
}

// OpenStream writes the stream open envelope and returns the live stream.
func OpenStream(ctx context.Context, sess PeerSession, open StreamOpen) (io.ReadWriteCloser, error) {
	if sess == nil {
		return nil, errors.New("nil peer session")
	}
	return sess.OpenStream(ctx, open)
}

// AcceptStream waits for the next logical stream.
func AcceptStream(ctx context.Context, sess PeerSession) (*AcceptedStream, error) {
	if sess == nil {
		return nil, errors.New("nil peer session")
	}
	return sess.AcceptStream(ctx)
}

// Dial upgrades the supplied PathResult into a live outbound peer session.
func Dial(ctx context.Context, cfg Config, result punch.PathResult) (PeerSession, error) {
	return upgrade(ctx, cfg, result, true)
}

// Accept upgrades the supplied PathResult into a live inbound peer session.
func Accept(ctx context.Context, cfg Config, result punch.PathResult) (PeerSession, error) {
	return upgrade(ctx, cfg, result, false)
}
