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
	"bytes"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"net"
	"strings"

	"github.com/miopunch/miopunch/connectivity"
	pocwire "github.com/miopunch/miopunch/internal/pocv1/wire"
	legacywire "github.com/miopunch/miopunch/internal/wire"
)

const (
	dialTagDialID           = 1
	dialTagPunchToken       = 2
	dialTagCandidates       = 3
	dialTagMemberCredential = 4
	dialTagUDPSnapshot      = 5
	dialTagUDPDecision      = 6
	dialTagP2PNetwork       = 7
	dialTagP2PIPFamily      = 8

	candidateTagKind = 1
	candidateTagAddr = 2
)

var dialAllowedTags = []uint64{
	dialTagDialID,
	dialTagPunchToken,
	dialTagCandidates,
	dialTagMemberCredential,
	dialTagUDPSnapshot,
	dialTagUDPDecision,
	dialTagP2PNetwork,
	dialTagP2PIPFamily,
}

var candidateAllowedTags = []uint64{
	candidateTagKind,
	candidateTagAddr,
}

func (o DialOffer) MarshalBinary() ([]byte, error) {
	normalized, err := normalizeDialOffer(o)
	if err != nil {
		return nil, err
	}
	return marshalDialMessage(
		normalized.DialID,
		normalized.PunchToken,
		normalized.Candidates,
		normalized.UDPSnapshot,
		nil,
		normalized.P2PNetwork,
		normalized.P2PIPFamily,
		normalized.MemberCredential,
	)
}

func UnmarshalDialOffer(data []byte) (DialOffer, error) {
	dialID, punchToken, candidates, udpSnapshot, _, p2pNetwork, p2pIPFamily, memberCredential, err := unmarshalDialMessage(data)
	if err != nil {
		return DialOffer{}, err
	}
	return normalizeDialOffer(DialOffer{
		DialID:           dialID,
		PunchToken:       punchToken,
		Candidates:       candidates,
		UDPSnapshot:      udpSnapshot,
		P2PNetwork:       p2pNetwork,
		P2PIPFamily:      p2pIPFamily,
		MemberCredential: memberCredential,
	})
}

func (a DialAnswer) MarshalBinary() ([]byte, error) {
	normalized, err := normalizeDialAnswer(a)
	if err != nil {
		return nil, err
	}
	return marshalDialMessage(
		normalized.DialID,
		normalized.PunchToken,
		normalized.Candidates,
		normalized.UDPSnapshot,
		&normalized.Decision,
		"",
		"",
		normalized.MemberCredential,
	)
}

func UnmarshalDialAnswer(data []byte) (DialAnswer, error) {
	dialID, punchToken, candidates, udpSnapshot, decision, _, _, memberCredential, err := unmarshalDialMessage(data)
	if err != nil {
		return DialAnswer{}, err
	}
	return normalizeDialAnswer(DialAnswer{
		DialID:           dialID,
		PunchToken:       punchToken,
		Candidates:       candidates,
		UDPSnapshot:      udpSnapshot,
		Decision:         decision,
		MemberCredential: memberCredential,
	})
}

func marshalDialMessage(
	dialID string,
	punchToken []byte,
	candidates []Candidate,
	udpSnapshot UDPSnapshot,
	decision *UDPDecision,
	p2pNetwork connectivity.P2PNetwork,
	p2pIPFamily connectivity.P2PIPFamily,
	memberCredential []byte,
) ([]byte, error) {
	out := make([]byte, 0, 256)
	var err error
	out, err = pocwire.AppendASCIIField(out, dialTagDialID, dialID)
	if err != nil {
		return nil, err
	}
	out = pocwire.AppendBytesField(out, dialTagPunchToken, punchToken)
	out = pocwire.AppendBytesField(out, dialTagCandidates, marshalCandidates(candidates))
	out = pocwire.AppendBytesField(out, dialTagMemberCredential, memberCredential)
	snapshotBytes, err := json.Marshal(udpSnapshot)
	if err != nil {
		return nil, err
	}
	out = pocwire.AppendBytesField(out, dialTagUDPSnapshot, snapshotBytes)
	if decision != nil {
		decisionBytes, err := json.Marshal(decision)
		if err != nil {
			return nil, err
		}
		out = pocwire.AppendBytesField(out, dialTagUDPDecision, decisionBytes)
	}
	if strings.TrimSpace(string(p2pNetwork)) != "" {
		out, err = pocwire.AppendASCIIField(out, dialTagP2PNetwork, string(p2pNetwork))
		if err != nil {
			return nil, err
		}
	}
	if strings.TrimSpace(string(p2pIPFamily)) != "" {
		out, err = pocwire.AppendASCIIField(out, dialTagP2PIPFamily, string(p2pIPFamily))
		if err != nil {
			return nil, err
		}
	}
	return out, nil
}

func unmarshalDialMessage(data []byte) (string, []byte, []Candidate, UDPSnapshot, UDPDecision, connectivity.P2PNetwork, connectivity.P2PIPFamily, []byte, error) {
	fields, err := pocwire.DecodeFieldsStrict(data, dialAllowedTags...)
	if err != nil {
		return "", nil, nil, UDPSnapshot{}, UDPDecision{}, "", "", nil, err
	}
	index := indexPocFields(fields)
	dialID, err := requireASCII(index, dialTagDialID, "dial_id")
	if err != nil {
		return "", nil, nil, UDPSnapshot{}, UDPDecision{}, "", "", nil, err
	}
	punchToken, err := requireBytes(index, dialTagPunchToken, "punch_token")
	if err != nil {
		return "", nil, nil, UDPSnapshot{}, UDPDecision{}, "", "", nil, err
	}
	candidateBytes, err := requireBytes(index, dialTagCandidates, "candidates")
	if err != nil {
		return "", nil, nil, UDPSnapshot{}, UDPDecision{}, "", "", nil, err
	}
	candidates, err := unmarshalCandidates(candidateBytes)
	if err != nil {
		return "", nil, nil, UDPSnapshot{}, UDPDecision{}, "", "", nil, err
	}
	udpSnapshotBytes, err := requireBytes(index, dialTagUDPSnapshot, "udp_snapshot")
	if err != nil {
		return "", nil, nil, UDPSnapshot{}, UDPDecision{}, "", "", nil, err
	}
	var udpSnapshot UDPSnapshot
	if err := json.Unmarshal(udpSnapshotBytes, &udpSnapshot); err != nil {
		return "", nil, nil, UDPSnapshot{}, UDPDecision{}, "", "", nil, fmt.Errorf("decode udp_snapshot: %w", err)
	}
	var decision UDPDecision
	if field, ok := index[dialTagUDPDecision]; ok {
		if err := json.Unmarshal(field.Value, &decision); err != nil {
			return "", nil, nil, UDPSnapshot{}, UDPDecision{}, "", "", nil, fmt.Errorf("decode udp_decision: %w", err)
		}
	}
	p2pNetwork, err := optionalP2PNetwork(index, dialTagP2PNetwork)
	if err != nil {
		return "", nil, nil, UDPSnapshot{}, UDPDecision{}, "", "", nil, err
	}
	p2pIPFamily, err := optionalP2PIPFamily(index, dialTagP2PIPFamily)
	if err != nil {
		return "", nil, nil, UDPSnapshot{}, UDPDecision{}, "", "", nil, err
	}
	memberCredential, err := requireBytes(index, dialTagMemberCredential, "member_credential")
	if err != nil {
		return "", nil, nil, UDPSnapshot{}, UDPDecision{}, "", "", nil, err
	}
	return dialID, punchToken, candidates, udpSnapshot, decision, p2pNetwork, p2pIPFamily, memberCredential, nil
}

func marshalCandidates(candidates []Candidate) []byte {
	out := make([]byte, 0, len(candidates)*32)
	for _, candidate := range candidates {
		entry := make([]byte, 0, 64)
		entry, _ = pocwire.AppendASCIIField(entry, candidateTagKind, string(candidate.Kind))
		entry, _ = pocwire.AppendASCIIField(entry, candidateTagAddr, candidate.Addr)
		out = appendLengthPrefixed(out, entry)
	}
	return out
}

func unmarshalCandidates(data []byte) ([]Candidate, error) {
	entries, err := decodeLengthPrefixedList(data)
	if err != nil {
		return nil, err
	}
	candidates := make([]Candidate, 0, len(entries))
	for i, entry := range entries {
		fields, err := pocwire.DecodeFieldsStrict(entry, candidateAllowedTags...)
		if err != nil {
			return nil, fmt.Errorf("candidate %d: %w", i, err)
		}
		index := indexPocFields(fields)
		kind, err := requireASCII(index, candidateTagKind, "kind")
		if err != nil {
			return nil, fmt.Errorf("candidate %d: %w", i, err)
		}
		addr, err := requireASCII(index, candidateTagAddr, "addr")
		if err != nil {
			return nil, fmt.Errorf("candidate %d: %w", i, err)
		}
		candidates = append(candidates, Candidate{
			Kind: CandidateKind(kind),
			Addr: addr,
		})
	}
	return normalizeOptionalCandidates(candidates)
}

func normalizeDialOffer(in DialOffer) (DialOffer, error) {
	dialID, err := pocwire.CanonicalizeMsgID(in.DialID)
	if err != nil {
		return DialOffer{}, fmt.Errorf("%w: canonicalize dial_id: %w", ErrInvalidOffer, err)
	}
	if len(in.PunchToken) != 16 {
		return DialOffer{}, fmt.Errorf("%w: invalid punch token length: %d", ErrInvalidOffer, len(in.PunchToken))
	}
	candidates, err := normalizeOptionalCandidates(in.Candidates)
	if err != nil {
		return DialOffer{}, fmt.Errorf("%w: %w", ErrInvalidOffer, err)
	}
	udpSnapshot, err := normalizeUDPSnapshot(in.UDPSnapshot)
	if err != nil {
		return DialOffer{}, fmt.Errorf("%w: %w", ErrInvalidOffer, err)
	}
	p2pNetwork, err := normalizeDialOfferP2PNetwork(in.P2PNetwork)
	if err != nil {
		return DialOffer{}, fmt.Errorf("%w: %w", ErrInvalidOffer, err)
	}
	p2pIPFamily, err := normalizeDialOfferP2PIPFamily(in.P2PIPFamily)
	if err != nil {
		return DialOffer{}, fmt.Errorf("%w: %w", ErrInvalidOffer, err)
	}
	if len(in.MemberCredential) == 0 {
		return DialOffer{}, fmt.Errorf("%w: missing member credential", ErrInvalidOffer)
	}
	return DialOffer{
		DialID:           dialID,
		PunchToken:       append([]byte(nil), in.PunchToken...),
		Candidates:       candidates,
		UDPSnapshot:      udpSnapshot,
		P2PNetwork:       p2pNetwork,
		P2PIPFamily:      p2pIPFamily,
		MemberCredential: append([]byte(nil), in.MemberCredential...),
	}, nil
}

func normalizeDialAnswer(in DialAnswer) (DialAnswer, error) {
	dialID, err := pocwire.CanonicalizeMsgID(in.DialID)
	if err != nil {
		return DialAnswer{}, fmt.Errorf("%w: canonicalize dial_id: %w", ErrInvalidAnswer, err)
	}
	if len(in.PunchToken) != 16 {
		return DialAnswer{}, fmt.Errorf("%w: invalid punch token length: %d", ErrInvalidAnswer, len(in.PunchToken))
	}
	candidates, err := normalizeOptionalCandidates(in.Candidates)
	if err != nil {
		return DialAnswer{}, fmt.Errorf("%w: %w", ErrInvalidAnswer, err)
	}
	udpSnapshot, err := normalizeUDPSnapshot(in.UDPSnapshot)
	if err != nil {
		return DialAnswer{}, fmt.Errorf("%w: %w", ErrInvalidAnswer, err)
	}
	decision, err := normalizeUDPDecision(in.Decision)
	if err != nil {
		return DialAnswer{}, fmt.Errorf("%w: %w", ErrInvalidAnswer, err)
	}
	if len(in.MemberCredential) == 0 {
		return DialAnswer{}, fmt.Errorf("%w: missing member credential", ErrInvalidAnswer)
	}
	return DialAnswer{
		DialID:           dialID,
		PunchToken:       append([]byte(nil), in.PunchToken...),
		Candidates:       candidates,
		UDPSnapshot:      udpSnapshot,
		Decision:         decision,
		MemberCredential: append([]byte(nil), in.MemberCredential...),
	}, nil
}

func normalizeUDPSnapshot(in UDPSnapshot) (UDPSnapshot, error) {
	directAddrs, err := normalizeUDPAddrStrings(in.DirectAddrs)
	if err != nil {
		return UDPSnapshot{}, fmt.Errorf("direct_addrs: %w", err)
	}
	mappedAddrs, err := normalizeUDPAddrStrings(in.MappedAddrs)
	if err != nil {
		return UDPSnapshot{}, fmt.Errorf("mapped_addrs: %w", err)
	}
	assistedAddrs, err := normalizeUDPAddrStrings(in.AssistedAddrs)
	if err != nil {
		return UDPSnapshot{}, fmt.Errorf("assisted_addrs: %w", err)
	}
	if len(directAddrs) == 0 && len(mappedAddrs) == 0 && len(assistedAddrs) == 0 {
		return UDPSnapshot{}, fmt.Errorf("udp snapshot empty")
	}
	return UDPSnapshot{
		DirectAddrs:   directAddrs,
		MappedAddrs:   mappedAddrs,
		AssistedAddrs: assistedAddrs,
	}, nil
}

func normalizeUDPAddrStrings(in []string) ([]string, error) {
	out := make([]string, 0, len(in))
	seen := make(map[string]struct{}, len(in))
	for i, addr := range in {
		addr = strings.TrimSpace(addr)
		if addr == "" {
			return nil, fmt.Errorf("addr %d empty", i)
		}
		resolved, err := net.ResolveUDPAddr("udp", addr)
		if err != nil {
			return nil, fmt.Errorf("addr %d resolve %q: %w", i, addr, err)
		}
		if resolved == nil || resolved.Port <= 0 {
			return nil, fmt.Errorf("addr %d invalid udp addr %q", i, addr)
		}
		normalized := resolved.String()
		if _, ok := seen[normalized]; ok {
			continue
		}
		seen[normalized] = struct{}{}
		out = append(out, normalized)
	}
	return out, nil
}

func normalizeUDPDecision(in UDPDecision) (UDPDecision, error) {
	if strings.TrimSpace(in.LocalResponse.Sid) == "" {
		return UDPDecision{}, fmt.Errorf("missing local response sid")
	}
	if strings.TrimSpace(in.RemoteResponse.Sid) == "" {
		return UDPDecision{}, fmt.Errorf("missing remote response sid")
	}
	return UDPDecision{
		LocalResponse:  cloneNatHoleResp(in.LocalResponse),
		RemoteResponse: cloneNatHoleResp(in.RemoteResponse),
		AnalysisKey:    strings.TrimSpace(in.AnalysisKey),
		AnalyzerKey:    strings.TrimSpace(in.AnalyzerKey),
		Mode:           in.Mode,
		Index:          in.Index,
	}, nil
}

func normalizeDialOfferP2PNetwork(value connectivity.P2PNetwork) (connectivity.P2PNetwork, error) {
	network, err := connectivity.ParseP2PNetwork(string(value))
	if err != nil {
		return "", err
	}
	switch network {
	case connectivity.P2PNetworkAuto, connectivity.P2PNetworkUDPOnly:
		return network, nil
	case connectivity.P2PNetworkTCPOnly:
		return "", ErrUnsupportedP2PNetwork
	default:
		return "", fmt.Errorf("invalid p2p network %q", network)
	}
}

func normalizeDialOfferP2PIPFamily(value connectivity.P2PIPFamily) (connectivity.P2PIPFamily, error) {
	return connectivity.ParseP2PIPFamily(string(value))
}

func cloneNatHoleResp(in legacywire.NatHoleResp) legacywire.NatHoleResp {
	out := in
	out.PeerDirectAddrs = append([]string(nil), in.PeerDirectAddrs...)
	out.CandidateAddrs = append([]string(nil), in.CandidateAddrs...)
	out.AssistedAddrs = append([]string(nil), in.AssistedAddrs...)
	out.DetectBehavior.CandidatePorts = append([]legacywire.PortsRange(nil), in.DetectBehavior.CandidatePorts...)
	return out
}

func indexPocFields(fields []pocwire.DecodedField) map[uint64]pocwire.DecodedField {
	out := make(map[uint64]pocwire.DecodedField, len(fields))
	for _, field := range fields {
		out[field.Tag] = field
	}
	return out
}

func requireASCII(index map[uint64]pocwire.DecodedField, tag uint64, name string) (string, error) {
	field, ok := index[tag]
	if !ok {
		return "", fmt.Errorf("%w: missing %s", pocwire.ErrInvalidFieldValue, name)
	}
	return pocwire.DecodeASCIIField(field)
}

func requireBytes(index map[uint64]pocwire.DecodedField, tag uint64, name string) ([]byte, error) {
	field, ok := index[tag]
	if !ok {
		return nil, fmt.Errorf("%w: missing %s", pocwire.ErrInvalidFieldValue, name)
	}
	return append([]byte(nil), field.Value...), nil
}

func optionalP2PNetwork(index map[uint64]pocwire.DecodedField, tag uint64) (connectivity.P2PNetwork, error) {
	field, ok := index[tag]
	if !ok {
		return connectivity.P2PNetworkAuto, nil
	}
	value, err := pocwire.DecodeASCIIField(field)
	if err != nil {
		return "", err
	}
	return connectivity.ParseP2PNetwork(value)
}

func optionalP2PIPFamily(index map[uint64]pocwire.DecodedField, tag uint64) (connectivity.P2PIPFamily, error) {
	field, ok := index[tag]
	if !ok {
		return connectivity.P2PIPFamilyAuto, nil
	}
	value, err := pocwire.DecodeASCIIField(field)
	if err != nil {
		return "", err
	}
	return connectivity.ParseP2PIPFamily(value)
}

func appendLengthPrefixed(dst []byte, value []byte) []byte {
	var buf [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(buf[:], uint64(len(value)))
	dst = append(dst, buf[:n]...)
	return append(dst, value...)
}

func decodeLengthPrefixedList(data []byte) ([][]byte, error) {
	out := make([][]byte, 0, 4)
	offset := 0
	for offset < len(data) {
		length, n, err := decodeCanonicalUvarint(data[offset:])
		if err != nil {
			return nil, err
		}
		offset += n
		if length > uint64(len(data)-offset) {
			return nil, fmt.Errorf("%w: candidates entry length %d exceeds remaining %d", pocwire.ErrTruncatedTLV, length, len(data)-offset)
		}
		out = append(out, append([]byte(nil), data[offset:offset+int(length)]...))
		offset += int(length)
	}
	return out, nil
}

func decodeCanonicalUvarint(data []byte) (uint64, int, error) {
	value, n := binary.Uvarint(data)
	switch {
	case n == 0:
		return 0, 0, fmt.Errorf("%w: truncated uvarint", pocwire.ErrTruncatedTLV)
	case n < 0:
		return 0, 0, fmt.Errorf("%w: overflow", pocwire.ErrNonCanonicalUvarint)
	}
	var buf [binary.MaxVarintLen64]byte
	m := binary.PutUvarint(buf[:], value)
	if m != n || !bytes.Equal(buf[:m], data[:n]) {
		return 0, 0, fmt.Errorf("%w", pocwire.ErrNonCanonicalUvarint)
	}
	return value, n, nil
}
