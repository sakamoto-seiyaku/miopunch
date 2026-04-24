package wire

import (
	"bytes"
	"slices"
	"testing"
)

func TestWriteReadMsg_RoundTrip(t *testing.T) {
	buf := bytes.NewBuffer(nil)
	in := &PeerHello{
		Role:       "client",
		User:       "u",
		ProxyName:  "p",
		SecretKey:  "s",
		AllowUsers: []string{"*"},
	}
	if err := WriteMsg(buf, in); err != nil {
		t.Fatalf("WriteMsg: %v", err)
	}

	out, err := ReadMsg(buf)
	if err != nil {
		t.Fatalf("ReadMsg: %v", err)
	}
	hello, ok := out.(*PeerHello)
	if !ok {
		t.Fatalf("unexpected type: %T", out)
	}
	if hello.Role != in.Role || hello.ProxyName != in.ProxyName {
		t.Fatalf("mismatch: %#v vs %#v", hello, in)
	}
}

func TestWriteReadMsg_RoundTrip_TCPFields(t *testing.T) {
	buf := bytes.NewBuffer(nil)
	in := &NatHoleVisitor{
		TransactionID: "tx",
		ProxyName:     "p",
		TCPDirectAddrs: []string{
			"192.0.2.1:1111",
		},
		TCPMappedAddrs: []string{
			"203.0.113.1:40000",
		},
		TCPSTUNCN: &STUNViewObservation{
			Available:     true,
			OkCount:       2,
			RTTMs:         10,
			NATDifficulty: 1,
			MappedAddrs: []string{
				"203.0.113.1:40000",
			},
		},
		TCPSTUNGlobal: &STUNViewObservation{
			Available:     false,
			NATDifficulty: 999,
		},
	}
	if err := WriteMsg(buf, in); err != nil {
		t.Fatalf("WriteMsg: %v", err)
	}

	out, err := ReadMsg(buf)
	if err != nil {
		t.Fatalf("ReadMsg: %v", err)
	}
	got, ok := out.(*NatHoleVisitor)
	if !ok {
		t.Fatalf("unexpected type: %T", out)
	}

	if !slices.Equal(got.TCPDirectAddrs, in.TCPDirectAddrs) {
		t.Fatalf("TCPDirectAddrs = %v, want %v", got.TCPDirectAddrs, in.TCPDirectAddrs)
	}
	if !slices.Equal(got.TCPMappedAddrs, in.TCPMappedAddrs) {
		t.Fatalf("TCPMappedAddrs = %v, want %v", got.TCPMappedAddrs, in.TCPMappedAddrs)
	}

	if got.TCPSTUNCN == nil {
		t.Fatalf("TCPSTUNCN = nil, want non-nil")
	}
	if got.TCPSTUNCN.Available != in.TCPSTUNCN.Available {
		t.Fatalf("TCPSTUNCN.Available = %v, want %v", got.TCPSTUNCN.Available, in.TCPSTUNCN.Available)
	}
	if !slices.Equal(got.TCPSTUNCN.MappedAddrs, in.TCPSTUNCN.MappedAddrs) {
		t.Fatalf("TCPSTUNCN.MappedAddrs = %v, want %v", got.TCPSTUNCN.MappedAddrs, in.TCPSTUNCN.MappedAddrs)
	}

	if got.TCPSTUNGlobal == nil {
		t.Fatalf("TCPSTUNGlobal = nil, want non-nil")
	}
	if got.TCPSTUNGlobal.Available != in.TCPSTUNGlobal.Available {
		t.Fatalf("TCPSTUNGlobal.Available = %v, want %v", got.TCPSTUNGlobal.Available, in.TCPSTUNGlobal.Available)
	}
	if got.TCPSTUNGlobal.NATDifficulty != in.TCPSTUNGlobal.NATDifficulty {
		t.Fatalf("TCPSTUNGlobal.NATDifficulty = %v, want %v", got.TCPSTUNGlobal.NATDifficulty, in.TCPSTUNGlobal.NATDifficulty)
	}
}
