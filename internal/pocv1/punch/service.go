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
)

// Dial runs the current v1 initiator-side dial/punch flow and returns the
// closed PathResult handoff for 05.
func Dial(ctx context.Context, cfg Config, target Target) (PathResult, error) {
	loaded, err := loadConfig(cfg)
	if err != nil {
		return PathResult{}, err
	}
	remote, err := resolveTarget(loaded, target)
	if err != nil {
		return PathResult{}, err
	}
	session, err := loaded.OpenPeerMessage(ctx, loaded)
	if err != nil {
		return PathResult{}, fmt.Errorf("open peer message session: %w", err)
	}
	defer session.Close()

	dialID, err := loaded.NewDialID()
	if err != nil {
		return PathResult{}, fmt.Errorf("new dial_id: %w", err)
	}
	punchToken, err := loaded.NewPunchToken()
	if err != nil {
		return PathResult{}, fmt.Errorf("new punch_token: %w", err)
	}
	offer := DialOffer{
		DialID:           dialID,
		PunchToken:       punchToken,
		Candidates:       append([]Candidate(nil), loaded.LocalCandidates...),
		MemberCredential: append([]byte(nil), loaded.SelfCredential...),
	}
	answer, verifiedRemote, _, err := exchangeOffer(ctx, loaded, session, remote, offer)
	if err != nil {
		return PathResult{}, err
	}
	return runPunch(ctx, loaded, verifiedRemote, dialID, punchToken, answer.Candidates, true)
}

// HandleOne waits for one inbound dial_offer, validates it, answers it, and
// returns the resulting PathResult.
func HandleOne(ctx context.Context, cfg Config) (PathResult, error) {
	loaded, err := loadConfig(cfg)
	if err != nil {
		return PathResult{}, err
	}
	session, err := loaded.OpenPeerMessage(ctx, loaded)
	if err != nil {
		return PathResult{}, fmt.Errorf("open peer message session: %w", err)
	}
	defer session.Close()
	return waitAndAnswerOffer(ctx, loaded, session)
}
