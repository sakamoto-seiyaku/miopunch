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

package punch

import (
	"context"
	"fmt"

	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/pocv1/peere2e"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
)

func exchangeOffer(
	ctx context.Context,
	cfg LoadedConfig,
	session peerMessageSession,
	target trustedRemote,
	offer DialOffer,
) (DialAnswer, trustedRemote, string, error) {
	body, err := offer.MarshalBinary()
	if err != nil {
		return DialAnswer{}, trustedRemote{}, "", err
	}
	logutil.Debugf(
		"punch offer publish: dial_id=%s target_peer_id=%s local_candidates=%s",
		offer.DialID,
		target.PeerID,
		formatCandidates(offer.Candidates),
	)
	msgID, err := cfg.NewMsgID()
	if err != nil {
		return DialAnswer{}, trustedRemote{}, "", fmt.Errorf("new msg_id: %w", err)
	}
	inboxTopic, err := cfg.TopicScope.InboxTopic(target.PeerID)
	if err != nil {
		return DialAnswer{}, trustedRemote{}, "", fmt.Errorf("derive target inbox topic: %w", err)
	}
	now := cfg.NowUnixMs()
	inner := pocwire.InnerMessage{
		DstPeerID:       target.PeerID,
		MsgID:           msgID,
		CreatedAtUnixMs: now,
		ExpiresAtUnixMs: now + uint64(defaultInnerTTL.Milliseconds()),
		SenderPeerID:    cfg.LocalPeerID,
		SenderEd25519:   append([]byte(nil), cfg.LocalEd25519Pub...),
		Kind:            pocwire.KindDialOffer,
		Body:            body,
	}
	if err := pocwire.SignInner(cfg.LocalEd25519Priv, &inner); err != nil {
		return DialAnswer{}, trustedRemote{}, "", fmt.Errorf("sign dial offer: %w", err)
	}
	if _, err := session.PublishInner(ctx, inboxTopic, inner, target.X25519PublicKey, peere2e.SealOptions{}); err != nil {
		return DialAnswer{}, trustedRemote{}, "", fmt.Errorf("publish dial offer: %w", err)
	}

	for {
		opened, err := session.WaitOpened(ctx, cfg.LocalX25519Priv, peere2e.OpenOptions{
			NowUnixMs: cfg.NowUnixMs(),
		})
		if err != nil {
			return DialAnswer{}, trustedRemote{}, "", fmt.Errorf("wait dial answer: %w", err)
		}
		if opened.Inner.Kind != pocwire.KindDialAnswer || opened.Inner.InReplyTo != msgID {
			continue
		}
		answer, remote, err := verifyAnswer(cfg, opened, offer.DialID, offer.PunchToken, msgID)
		if err != nil {
			return DialAnswer{}, trustedRemote{}, "", err
		}
		if remote.PeerID != target.PeerID {
			return DialAnswer{}, trustedRemote{}, "", fmt.Errorf("%w: remote peer_id mismatch", ErrInvalidAnswer)
		}
		logutil.Debugf(
			"punch answer received: dial_id=%s target_peer_id=%s remote_candidates=%s",
			answer.DialID,
			remote.PeerID,
			formatCandidates(answer.Candidates),
		)
		return answer, remote, msgID, nil
	}
}

func waitAndAnswerOffer(ctx context.Context, cfg LoadedConfig, session peerMessageSession) (PathResult, error) {
	for {
		opened, err := session.WaitOpened(ctx, cfg.LocalX25519Priv, peere2e.OpenOptions{
			NowUnixMs: cfg.NowUnixMs(),
		})
		if err != nil {
			return PathResult{}, fmt.Errorf("wait dial offer: %w", err)
		}
		currentCfg, err := reloadOfferConfig(cfg)
		if err != nil {
			return PathResult{}, err
		}
		offer, remote, err := verifyOffer(currentCfg, opened)
		if err != nil {
			continue
		}
		logutil.Debugf(
			"punch offer received: dial_id=%s remote_peer_id=%s remote_candidates=%s",
			offer.DialID,
			remote.PeerID,
			formatCandidates(offer.Candidates),
		)
		answer := DialAnswer{
			DialID:           offer.DialID,
			PunchToken:       append([]byte(nil), offer.PunchToken...),
			Candidates:       append([]Candidate(nil), currentCfg.LocalCandidates...),
			MemberCredential: append([]byte(nil), currentCfg.SelfCredential...),
		}
		body, err := answer.MarshalBinary()
		if err != nil {
			return PathResult{}, err
		}
		replyMsgID, err := currentCfg.NewMsgID()
		if err != nil {
			return PathResult{}, fmt.Errorf("new msg_id: %w", err)
		}
		replyTopic, err := currentCfg.TopicScope.InboxTopic(remote.PeerID)
		if err != nil {
			return PathResult{}, fmt.Errorf("derive reply inbox topic: %w", err)
		}
		now := currentCfg.NowUnixMs()
		inner := pocwire.InnerMessage{
			DstPeerID:       remote.PeerID,
			MsgID:           replyMsgID,
			CreatedAtUnixMs: now,
			ExpiresAtUnixMs: now + uint64(defaultInnerTTL.Milliseconds()),
			SenderPeerID:    currentCfg.LocalPeerID,
			SenderEd25519:   append([]byte(nil), currentCfg.LocalEd25519Pub...),
			Kind:            pocwire.KindDialAnswer,
			InReplyTo:       opened.Inner.MsgID,
			Body:            body,
		}
		if err := pocwire.SignInner(currentCfg.LocalEd25519Priv, &inner); err != nil {
			return PathResult{}, fmt.Errorf("sign dial answer: %w", err)
		}
		if _, err := session.PublishInner(ctx, replyTopic, inner, remote.X25519PublicKey, peere2e.SealOptions{}); err != nil {
			return PathResult{}, fmt.Errorf("publish dial answer: %w", err)
		}
		logutil.Debugf(
			"punch answer publish: dial_id=%s remote_peer_id=%s local_candidates=%s",
			answer.DialID,
			remote.PeerID,
			formatCandidates(answer.Candidates),
		)
		return runPunch(ctx, currentCfg, remote, offer.DialID, offer.PunchToken, offer.Candidates, false)
	}
}

func reloadOfferConfig(cfg LoadedConfig) (LoadedConfig, error) {
	rosterSnapshot, err := cfg.Store.LoadRosterSnapshot(cfg.NetworkID)
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("reload roster snapshot: %w", err)
	}

	trustedRosterByID := make(map[string]persist.RosterEntry, len(rosterSnapshot.Entries))
	for _, entry := range rosterSnapshot.Entries {
		trustedRosterByID[entry.PeerID] = entry
	}

	cfg.RosterSnapshot = rosterSnapshot
	cfg.TrustedRosterByID = trustedRosterByID
	return cfg, nil
}
