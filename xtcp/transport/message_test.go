package transport

import (
	"context"
	"sync"
	"testing"
	"time"

	"github.com/miopunch/miopunch/xtcp/msg"
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

func TestMessageTransporter_DoDispatch(t *testing.T) {
	sender := &recorderSender{}
	tr := NewMessageTransporter(sender)

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	lane := "tx-1"
	req := &msg.NatHoleVisitor{TransactionID: lane, ProxyName: "p", PreCheck: true}

	done := make(chan error, 1)
	go func() {
		_, err := tr.Do(ctx, req, lane, msg.TypeNameNatHoleResp)
		done <- err
	}()

	time.Sleep(50 * time.Millisecond)
	resp := &msg.NatHoleResp{TransactionID: lane}
	if !tr.DispatchWithType(resp, msg.TypeNameNatHoleResp, lane) {
		t.Fatalf("DispatchWithType returned false")
	}

	if err := <-done; err != nil {
		t.Fatalf("Do returned error: %v", err)
	}
}
