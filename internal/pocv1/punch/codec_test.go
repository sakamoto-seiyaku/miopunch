package punch

import (
	"bytes"
	"errors"
	"testing"

	"github.com/miopunch/miopunch/connectivity"
)

func TestDialOfferRoundTrip(t *testing.T) {
	offer := DialOffer{
		DialID:     mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA"),
		PunchToken: bytes.Repeat([]byte{0x42}, 16),
		Candidates: []Candidate{
			{Kind: CandidateKindHost, Addr: "127.0.0.1:4000"},
			{Kind: CandidateKindSrflx, Addr: "203.0.113.10:5000"},
		},
		UDPSnapshot:      testUDPSnapshot("127.0.0.1:4000"),
		P2PNetwork:       connectivity.P2PNetworkUDPOnly,
		P2PIPFamily:      connectivity.P2PIPFamilyV4,
		MemberCredential: []byte{0x01, 0x02, 0x03},
	}
	data, err := offer.MarshalBinary()
	if err != nil {
		t.Fatalf("DialOffer.MarshalBinary() error = %v, want nil", err)
	}
	got, err := UnmarshalDialOffer(data)
	if err != nil {
		t.Fatalf("UnmarshalDialOffer() error = %v, want nil", err)
	}
	if got.DialID != offer.DialID {
		t.Fatalf("UnmarshalDialOffer().DialID = %q, want %q", got.DialID, offer.DialID)
	}
	if !bytes.Equal(got.PunchToken, offer.PunchToken) {
		t.Fatalf("UnmarshalDialOffer().PunchToken = %x, want %x", got.PunchToken, offer.PunchToken)
	}
	if !bytes.Equal(got.MemberCredential, offer.MemberCredential) {
		t.Fatalf("UnmarshalDialOffer().MemberCredential = %x, want %x", got.MemberCredential, offer.MemberCredential)
	}
	if got.P2PNetwork != offer.P2PNetwork {
		t.Fatalf("UnmarshalDialOffer().P2PNetwork = %q, want %q", got.P2PNetwork, offer.P2PNetwork)
	}
	if got.P2PIPFamily != offer.P2PIPFamily {
		t.Fatalf("UnmarshalDialOffer().P2PIPFamily = %q, want %q", got.P2PIPFamily, offer.P2PIPFamily)
	}
	if len(got.Candidates) != len(offer.Candidates) {
		t.Fatalf("UnmarshalDialOffer().Candidates length = %d, want %d", len(got.Candidates), len(offer.Candidates))
	}
}

func TestDialOfferOldWireDefaultsPolicyToAuto(t *testing.T) {
	data, err := marshalDialMessage(
		mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA"),
		bytes.Repeat([]byte{0x42}, 16),
		[]Candidate{{Kind: CandidateKindHost, Addr: "127.0.0.1:4000"}},
		testUDPSnapshot("127.0.0.1:4000"),
		nil,
		"",
		"",
		[]byte{0x01, 0x02, 0x03},
	)
	if err != nil {
		t.Fatalf("marshalDialMessage() error = %v, want nil", err)
	}

	got, err := UnmarshalDialOffer(data)
	if err != nil {
		t.Fatalf("UnmarshalDialOffer(old wire) error = %v, want nil", err)
	}
	if got.P2PNetwork != connectivity.P2PNetworkAuto {
		t.Fatalf("UnmarshalDialOffer(old wire).P2PNetwork = %q, want %q", got.P2PNetwork, connectivity.P2PNetworkAuto)
	}
	if got.P2PIPFamily != connectivity.P2PIPFamilyAuto {
		t.Fatalf("UnmarshalDialOffer(old wire).P2PIPFamily = %q, want %q", got.P2PIPFamily, connectivity.P2PIPFamilyAuto)
	}
}

func TestDialOfferAllowsEmptyCandidatesWhenSnapshotHasMaterial(t *testing.T) {
	offer := DialOffer{
		DialID:           mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA"),
		PunchToken:       bytes.Repeat([]byte{0x42}, 16),
		UDPSnapshot:      testUDPSnapshot("192.0.2.10:4000"),
		P2PNetwork:       connectivity.P2PNetworkUDPOnly,
		P2PIPFamily:      connectivity.P2PIPFamilyV4,
		MemberCredential: []byte{0x01, 0x02, 0x03},
	}
	data, err := offer.MarshalBinary()
	if err != nil {
		t.Fatalf("DialOffer.MarshalBinary(empty candidates) error = %v, want nil", err)
	}
	got, err := UnmarshalDialOffer(data)
	if err != nil {
		t.Fatalf("UnmarshalDialOffer(empty candidates) error = %v, want nil", err)
	}
	if len(got.Candidates) != 0 {
		t.Fatalf("UnmarshalDialOffer(empty candidates).Candidates = %#v, want empty", got.Candidates)
	}
	if len(got.UDPSnapshot.DirectAddrs) != 1 {
		t.Fatalf("UnmarshalDialOffer(empty candidates).UDPSnapshot.DirectAddrs = %#v, want one addr", got.UDPSnapshot.DirectAddrs)
	}
}

func TestDialAnswerRoundTrip(t *testing.T) {
	answer := DialAnswer{
		DialID:     mustCanonicalMsgID(t, "MFRGGZDFMZTWQ2LKNNWG23TPOI"),
		PunchToken: bytes.Repeat([]byte{0x99}, 16),
		Candidates: []Candidate{
			{Kind: CandidateKindHost, Addr: "127.0.0.1:4100"},
		},
		UDPSnapshot:      testUDPSnapshot("127.0.0.1:4100"),
		Decision:         testUDPDecision(mustCanonicalMsgID(t, "MFRGGZDFMZTWQ2LKNNWG23TPOI"), "127.0.0.1:4000", "127.0.0.1:4100"),
		MemberCredential: []byte{0x05, 0x06},
	}
	data, err := answer.MarshalBinary()
	if err != nil {
		t.Fatalf("DialAnswer.MarshalBinary() error = %v, want nil", err)
	}
	if !bytes.Contains(data, []byte("decision_mode")) {
		t.Fatalf("DialAnswer.MarshalBinary() missing decision_mode field")
	}
	if !bytes.Contains(data, []byte("decision_index")) {
		t.Fatalf("DialAnswer.MarshalBinary() missing decision_index field")
	}
	if bytes.Contains(data, []byte(`"mode"`)) {
		t.Fatalf("DialAnswer.MarshalBinary() contains legacy mode field")
	}
	if bytes.Contains(data, []byte(`"index"`)) {
		t.Fatalf("DialAnswer.MarshalBinary() contains legacy index field")
	}
	got, err := UnmarshalDialAnswer(data)
	if err != nil {
		t.Fatalf("UnmarshalDialAnswer() error = %v, want nil", err)
	}
	if got.DialID != answer.DialID {
		t.Fatalf("UnmarshalDialAnswer().DialID = %q, want %q", got.DialID, answer.DialID)
	}
	if !bytes.Equal(got.PunchToken, answer.PunchToken) {
		t.Fatalf("UnmarshalDialAnswer().PunchToken = %x, want %x", got.PunchToken, answer.PunchToken)
	}
}

func TestDialAnswerRejectsMissingUDPDecision(t *testing.T) {
	dialID := mustCanonicalMsgID(t, "MFRGGZDFMZTWQ2LKNNWG23TPOI")
	data, err := marshalDialMessage(
		dialID,
		bytes.Repeat([]byte{0x99}, 16),
		[]Candidate{{Kind: CandidateKindHost, Addr: "127.0.0.1:4100"}},
		testUDPSnapshot("127.0.0.1:4100"),
		nil,
		"",
		"",
		[]byte{0x05, 0x06},
	)
	if err != nil {
		t.Fatalf("marshalDialMessage() error = %v, want nil", err)
	}

	if _, err := UnmarshalDialAnswer(data); !errors.Is(err, ErrInvalidAnswer) {
		t.Fatalf("UnmarshalDialAnswer(missing udp_decision) error = %v, want %v", err, ErrInvalidAnswer)
	}
}
