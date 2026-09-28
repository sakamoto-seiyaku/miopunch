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

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/pocv1/peere2e"
	"github.com/miopunch/miopunch/internal/pocv1/persist"
	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
	"github.com/miopunch/miopunch/internal/punchdecision"
	legacywire "github.com/miopunch/miopunch/internal/wire"
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
	logUDPSnapshot("punch offer publish udp snapshot", offer.DialID, offer.UDPSnapshot)
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
		logUDPSnapshot("punch answer received udp snapshot", answer.DialID, answer.UDPSnapshot)
		logUDPDecision("punch answer received udp decision", answer.DialID, answer.Decision)
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
		logUDPSnapshot("punch offer received udp snapshot", offer.DialID, offer.UDPSnapshot)
		offerCfg, err := configForDialOfferPolicy(currentCfg, offer)
		if err != nil {
			return PathResult{}, err
		}
		logutil.Debugf(
			"punch offer policy applied: dial_id=%s remote_peer_id=%s p2p_network=%s p2p_ip_family=%s local_candidates=%s",
			offer.DialID,
			remote.PeerID,
			offerCfg.P2PNetwork,
			offerCfg.P2PIPFamily,
			formatCandidates(offerCfg.LocalCandidates),
		)
		peerDirectAddrs := peerDirectAddrStrings(offer.UDPSnapshot, offer.Candidates)
		udpSnapshot, err := offerCfg.GatherUDPSnapshot(ctx, offerCfg, offer.DialID)
		if err != nil {
			fallbackCfg, fallbackSnapshot := augmentLocalPathMaterialFromPeerDirect(
				offerCfg,
				UDPSnapshot{},
				peerDirectAddrs,
				offer.DialID,
				"punch answer fallback",
			)
			if len(fallbackSnapshot.DirectAddrs) == 0 && len(fallbackSnapshot.AssistedAddrs) == 0 {
				return PathResult{}, err
			}
			logutil.Debugf("punch answer snapshot recovered by route-source: dial_id=%s gather_err=%v", offer.DialID, err)
			offerCfg = fallbackCfg
			udpSnapshot = fallbackSnapshot
		}
		offerCfg, udpSnapshot = augmentLocalPathMaterialFromPeerDirect(
			offerCfg,
			udpSnapshot,
			peerDirectAddrs,
			offer.DialID,
			"punch answer",
		)
		logUDPSnapshot("punch answer local udp snapshot", offer.DialID, udpSnapshot)
		decision, err := decideUDP(offer.DialID, remote.PeerID, offer.UDPSnapshot, udpSnapshot)
		if err != nil {
			return PathResult{}, err
		}
		logUDPDecision("punch answer local udp decision", offer.DialID, decision)
		answer := DialAnswer{
			DialID:           offer.DialID,
			PunchToken:       append([]byte(nil), offer.PunchToken...),
			Candidates:       append([]Candidate(nil), offerCfg.LocalCandidates...),
			UDPSnapshot:      udpSnapshot,
			Decision:         decision,
			MemberCredential: append([]byte(nil), offerCfg.SelfCredential...),
		}
		body, err := answer.MarshalBinary()
		if err != nil {
			return PathResult{}, err
		}
		replyMsgID, err := offerCfg.NewMsgID()
		if err != nil {
			return PathResult{}, fmt.Errorf("new msg_id: %w", err)
		}
		replyTopic, err := offerCfg.TopicScope.InboxTopic(remote.PeerID)
		if err != nil {
			return PathResult{}, fmt.Errorf("derive reply inbox topic: %w", err)
		}
		now := offerCfg.NowUnixMs()
		inner := pocwire.InnerMessage{
			DstPeerID:       remote.PeerID,
			MsgID:           replyMsgID,
			CreatedAtUnixMs: now,
			ExpiresAtUnixMs: now + uint64(defaultInnerTTL.Milliseconds()),
			SenderPeerID:    offerCfg.LocalPeerID,
			SenderEd25519:   append([]byte(nil), offerCfg.LocalEd25519Pub...),
			Kind:            pocwire.KindDialAnswer,
			InReplyTo:       opened.Inner.MsgID,
			Body:            body,
		}
		if err := pocwire.SignInner(offerCfg.LocalEd25519Priv, &inner); err != nil {
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
		logUDPSnapshot("punch answer publish udp snapshot", answer.DialID, answer.UDPSnapshot)
		return runPunch(ctx, offerCfg, remote, offer.DialID, offer.PunchToken, &answer.Decision.LocalResponse, answer.Decision, false)
	}
}

func configForDialOfferPolicy(cfg LoadedConfig, offer DialOffer) (LoadedConfig, error) {
	p2pNetwork, err := normalizePOCV1P2PNetwork(offer.P2PNetwork)
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("dial offer p2p_network: %w", err)
	}
	p2pIPFamily, err := normalizePOCV1P2PIPFamily(offer.P2PIPFamily, cfg.UDP6Conn)
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("dial offer p2p_ip_family: %w", err)
	}
	localCandidates, err := normalizeOptionalCandidatesForIPFamily(cfg.LocalCandidates, p2pIPFamily)
	if err != nil {
		return LoadedConfig{}, fmt.Errorf("dial offer local candidates: %w", err)
	}

	out := cfg
	out.P2PNetwork = p2pNetwork
	out.P2PIPFamily = p2pIPFamily
	out.LocalCandidates = localCandidates
	return out, nil
}

func decideUDP(dialID string, remotePeerID string, remoteSnapshot UDPSnapshot, localSnapshot UDPSnapshot) (UDPDecision, error) {
	logUDPSnapshot("punch decision remote udp snapshot", dialID, remoteSnapshot)
	logUDPSnapshot("punch decision local udp snapshot", dialID, localSnapshot)
	res, err := punchdecision.AnalyzeWithDaemonMemory(
		dialID,
		remotePeerID,
		visitorSnapshot(dialID, remoteSnapshot, connectivity.P2PNetworkUDPOnly),
		clientSnapshot(dialID, localSnapshot, connectivity.P2PNetworkUDPOnly),
	)
	if err != nil {
		return UDPDecision{}, fmt.Errorf("decide udp punch: %w", err)
	}
	if res == nil || res.VisitorResponse == nil || res.ClientResponse == nil {
		return UDPDecision{}, fmt.Errorf("decide udp punch: incomplete decision result")
	}
	decision := UDPDecision{
		LocalResponse:  cloneNatHoleResp(*res.ClientResponse),
		RemoteResponse: cloneNatHoleResp(*res.VisitorResponse),
		AnalysisKey:    res.AnalysisKey,
		AnalyzerKey:    res.AnalyzerKey,
		Mode:           res.Mode,
		Index:          res.Index,
	}
	logUDPDecision("punch decision result", dialID, decision)
	return decision, nil
}

func logUDPSnapshot(label string, dialID string, snapshot UDPSnapshot) {
	logutil.Debugf(
		"%s: dial_id=%s direct_addrs=%v mapped_addrs=%v assisted_addrs=%v",
		label,
		dialID,
		snapshot.DirectAddrs,
		snapshot.MappedAddrs,
		snapshot.AssistedAddrs,
	)
}

func logUDPDecision(label string, dialID string, decision UDPDecision) {
	logNatHoleResp(label+" local_response", dialID, decision.LocalResponse)
	logNatHoleResp(label+" remote_response", dialID, decision.RemoteResponse)
	logutil.Debugf(
		"%s metadata: dial_id=%s analysis_key=%s analyzer_key=%s mode=%d index=%d",
		label,
		dialID,
		decision.AnalysisKey,
		decision.AnalyzerKey,
		decision.Mode,
		decision.Index,
	)
}

func logNatHoleResp(label string, dialID string, resp legacywire.NatHoleResp) {
	logutil.Debugf(
		"%s: dial_id=%s sid=%s peer_direct_addrs=%v candidate_addrs=%v assisted_addrs=%v punching_enabled=%t punching_error=%q detect_behavior=%+v p2p_network=%s",
		label,
		dialID,
		resp.Sid,
		resp.PeerDirectAddrs,
		resp.CandidateAddrs,
		resp.AssistedAddrs,
		resp.PunchingEnabled,
		resp.PunchingError,
		resp.DetectBehavior,
		resp.P2PNetwork,
	)
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
