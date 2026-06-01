package punch

import (
	"bytes"
	"testing"
)

func TestDialOfferRoundTrip(t *testing.T) {
	offer := DialOffer{
		DialID:     mustCanonicalMsgID(t, "JBSWY3DPEHPK3PXPJBSWY3DPAA"),
		PunchToken: bytes.Repeat([]byte{0x42}, 16),
		Candidates: []Candidate{
			{Kind: CandidateKindHost, Addr: "127.0.0.1:4000"},
			{Kind: CandidateKindSrflx, Addr: "203.0.113.10:5000"},
		},
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
	if len(got.Candidates) != len(offer.Candidates) {
		t.Fatalf("UnmarshalDialOffer().Candidates length = %d, want %d", len(got.Candidates), len(offer.Candidates))
	}
}

func TestDialAnswerRoundTrip(t *testing.T) {
	answer := DialAnswer{
		DialID:     mustCanonicalMsgID(t, "MFRGGZDFMZTWQ2LKNNWG23TPOI"),
		PunchToken: bytes.Repeat([]byte{0x99}, 16),
		Candidates: []Candidate{
			{Kind: CandidateKindHost, Addr: "127.0.0.1:4100"},
		},
		MemberCredential: []byte{0x05, 0x06},
	}
	data, err := answer.MarshalBinary()
	if err != nil {
		t.Fatalf("DialAnswer.MarshalBinary() error = %v, want nil", err)
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
