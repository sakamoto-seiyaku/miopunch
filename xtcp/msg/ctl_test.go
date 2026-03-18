package msg

import (
	"bytes"
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
