package nathole

import (
	"context"
	"sync"
	"testing"
	"time"

	"github.com/miopunch/miopunch/xtcp/msg"
	"github.com/miopunch/miopunch/xtcp/transport"
)

type recorderSender struct {
	mu   sync.Mutex
	sent []msg.Message
}

func (r *recorderSender) Send(m msg.Message) error {
	r.mu.Lock()
	r.sent = append(r.sent, m)
	r.mu.Unlock()
	return nil
}

func TestExchangeInfo_AllowsDirectOnlyWhenPunchingDisabled(t *testing.T) {
	sender := &recorderSender{}
	tr := transport.NewMessageTransporter(sender)

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	lane := "tx-1"
	req := &msg.NatHoleClient{TransactionID: lane, ProxyName: "p", Sid: "s"}

	done := make(chan struct{})
	var (
		got *msg.NatHoleResp
		err error
	)
	go func() {
		got, err = ExchangeInfo(ctx, tr, lane, req, 2*time.Second)
		close(done)
	}()

	time.Sleep(50 * time.Millisecond)
	resp := &msg.NatHoleResp{
		TransactionID:   lane,
		Sid:             "s",
		PeerDirectAddrs: []string{"[::1]:1234"},
		PunchingEnabled: false,
		PunchingError:   "stun missing",
	}
	if !tr.DispatchWithType(resp, msg.TypeNameNatHoleResp, lane) {
		t.Fatalf("DispatchWithType returned false")
	}

	<-done
	if err != nil {
		t.Fatalf("ExchangeInfo error: %v", err)
	}
	if got == nil || len(got.PeerDirectAddrs) != 1 || got.PeerDirectAddrs[0] != "[::1]:1234" {
		t.Fatalf("unexpected resp: %#v", got)
	}
}

func TestExchangeInfo_PunchingEnabledCompatWhenCandidateAddrsPresent(t *testing.T) {
	sender := &recorderSender{}
	tr := transport.NewMessageTransporter(sender)

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	lane := "tx-2"
	req := &msg.NatHoleClient{TransactionID: lane, ProxyName: "p", Sid: "s"}

	done := make(chan struct{})
	var (
		got *msg.NatHoleResp
		err error
	)
	go func() {
		got, err = ExchangeInfo(ctx, tr, lane, req, 2*time.Second)
		close(done)
	}()

	time.Sleep(50 * time.Millisecond)
	resp := &msg.NatHoleResp{
		TransactionID:  lane,
		Sid:            "s",
		CandidateAddrs: []string{"203.0.113.1:12345"},
	}
	if !tr.DispatchWithType(resp, msg.TypeNameNatHoleResp, lane) {
		t.Fatalf("DispatchWithType returned false")
	}

	<-done
	if err != nil {
		t.Fatalf("ExchangeInfo error: %v", err)
	}
	if got == nil || !got.PunchingEnabled {
		t.Fatalf("expected PunchingEnabled=true, got %#v", got)
	}
}
