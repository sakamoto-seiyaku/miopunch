package mqtt

import (
	"context"
	"sync"
	"testing"
	"time"

	"github.com/256dpi/gomqtt/broker"
	"github.com/256dpi/gomqtt/transport"

	"github.com/miopunch/miopunch/internal/wire"
)

func TestServeClient_TwoConcurrentVisitorsDoNotStomp(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	server, err := transport.Launch("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("transport.Launch error = %v, want nil", err)
	}

	backend := broker.NewMemoryBackend()
	engine := broker.NewEngine(backend)
	engine.Accept(server)
	defer func() {
		_ = server.Close()
		engine.Close()
	}()

	brokerURL := "tcp://" + server.Addr().String()

	sid := "sid-test"
	topicPrefix := "miopunch/test"

	openCfg := func(role Role) Config {
		return Config{
			BrokerURL:       brokerURL,
			TopicPrefix:     topicPrefix,
			SID:             sid,
			Role:            role,
			HelloInterval:   25 * time.Millisecond,
			HelloTimeout:    750 * time.Millisecond,
			ExchangeTimeout: 2 * time.Second,
			BarrierTimeout:  2 * time.Second,
			StartDelay:      50 * time.Millisecond,
		}
	}

	clientSess, err := Open(ctx, openCfg(RoleClient))
	if err != nil {
		t.Fatalf("Open(client) error = %v, want nil", err)
	}
	defer clientSess.Close()

	attemptCh := make(chan ClientAttempt, 8)
	clientTemplate := &wire.NatHoleClient{
		TransactionID: "template-tx",
		ProxyName:     "proxy",
		Sid:           sid,
		Protocol:      "quic",
		P2PNetwork:    "udp_only",
	}

	clientRunCtx, clientRunCancel := context.WithCancel(ctx)
	defer clientRunCancel()

	clientErrCh := make(chan error, 1)
	go func() {
		clientErrCh <- clientSess.ServeClient(clientRunCtx, clientTemplate, func(a ClientAttempt) {
			attemptCh <- a
		})
	}()

	visitorDial := func(tx string, startDelay time.Duration) error {
		cfg := openCfg(RoleVisitor)
		cfg.StartDelay = startDelay
		vs, err := Open(ctx, cfg)
		if err != nil {
			return err
		}
		defer vs.Close()

		_, err = vs.RunVisitor(ctx, &wire.NatHoleVisitor{
			TransactionID: tx,
			ProxyName:     "proxy",
			Protocol:      "quic",
			P2PNetwork:    "udp_only",
		}, func(sid string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*wire.NatHoleResp, *wire.NatHoleResp, error) {
			// The visitor publishes both resp/visitor and resp/client. The client
			// side only waits for resp/client; keeping the transaction_id aligned
			// with info/client makes correlation deterministic.
			return &wire.NatHoleResp{
					TransactionID: visitor.TransactionID,
					Sid:           sid,
					Protocol:      "quic",
				}, &wire.NatHoleResp{
					TransactionID: client.TransactionID,
					Sid:           sid,
					Protocol:      "quic",
				}, nil
		})
		return err
	}

	var wg sync.WaitGroup
	tx1 := "dial-1"
	tx2 := "dial-2"
	wg.Add(2)
	errs := make(chan error, 2)
	go func() { defer wg.Done(); errs <- visitorDial(tx1, 50*time.Millisecond) }()
	go func() { defer wg.Done(); errs <- visitorDial(tx2, 400*time.Millisecond) }()
	wg.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Fatalf("visitor dial error = %v, want nil", err)
		}
	}

	got := map[string]ClientAttempt{}
	for len(got) < 2 {
		select {
		case a := <-attemptCh:
			if a.Err != nil {
				t.Fatalf("ServeClient attempt error = %v, want nil", a.Err)
			}
			if a.DialID == "" {
				t.Fatalf("ServeClient attempt dial_id empty")
			}
			if a.ClientResp == nil {
				t.Fatalf("ServeClient attempt resp nil")
			}
			got[a.DialID] = a
		case err := <-clientErrCh:
			t.Fatalf("ServeClient returned early: %v", err)
		case <-ctx.Done():
			t.Fatalf("timed out waiting for attempts")
		}
	}

	if _, ok := got[tx1]; !ok {
		t.Fatalf("missing attempt for %q, got %v", tx1, keys(got))
	}
	if _, ok := got[tx2]; !ok {
		t.Fatalf("missing attempt for %q, got %v", tx2, keys(got))
	}

	if got[tx1].StartedAtMs <= 0 || got[tx2].StartedAtMs <= 0 {
		t.Fatalf("started_at_ms must be populated, got=%d/%d", got[tx1].StartedAtMs, got[tx2].StartedAtMs)
	}
	diff := got[tx2].StartedAtMs - got[tx1].StartedAtMs
	if diff < 150 {
		t.Fatalf("expected start windows to differ (bucketed), started_at_ms diff=%dms", diff)
	}
}

func TestServeClient_MaxActiveAttempts_DropsExcessDialIDs(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	server, err := transport.Launch("tcp://127.0.0.1:0")
	if err != nil {
		t.Fatalf("transport.Launch error = %v, want nil", err)
	}

	backend := broker.NewMemoryBackend()
	engine := broker.NewEngine(backend)
	engine.Accept(server)
	defer func() {
		_ = server.Close()
		engine.Close()
	}()

	brokerURL := "tcp://" + server.Addr().String()

	sid := "sid-test"
	topicPrefix := "miopunch/test"

	openCfg := func(role Role) Config {
		return Config{
			BrokerURL:         brokerURL,
			TopicPrefix:       topicPrefix,
			SID:               sid,
			Role:              role,
			HelloInterval:     25 * time.Millisecond,
			HelloTimeout:      750 * time.Millisecond,
			ExchangeTimeout:   2 * time.Second,
			BarrierTimeout:    2 * time.Second,
			StartDelay:        500 * time.Millisecond,
			MaxActiveAttempts: 1,
		}
	}

	clientSess, err := Open(ctx, openCfg(RoleClient))
	if err != nil {
		t.Fatalf("Open(client) error = %v, want nil", err)
	}
	defer clientSess.Close()

	clientTemplate := &wire.NatHoleClient{
		TransactionID: "template-tx",
		ProxyName:     "proxy",
		Sid:           sid,
		Protocol:      "quic",
		P2PNetwork:    "udp_only",
	}

	attemptCh := make(chan ClientAttempt, 8)
	clientRunCtx, clientRunCancel := context.WithCancel(ctx)
	defer clientRunCancel()

	clientErrCh := make(chan error, 1)
	go func() {
		clientErrCh <- clientSess.ServeClient(clientRunCtx, clientTemplate, func(a ClientAttempt) {
			attemptCh <- a
		})
	}()

	visitorSess, err := Open(ctx, openCfg(RoleVisitor))
	if err != nil {
		t.Fatalf("Open(visitor) error = %v, want nil", err)
	}
	defer visitorSess.Close()

	dial1 := "dial-1"
	dial2 := "dial-2"

	pub1Ctx, cancelPub1 := context.WithTimeout(ctx, 2*time.Second)
	defer cancelPub1()
	if err := visitorSess.publishJSON(pub1Ctx, visitorSess.attemptTopic(dial1, "info/visitor"), &wire.NatHoleVisitor{
		TransactionID: dial1,
		ProxyName:     "proxy",
		Protocol:      "quic",
		P2PNetwork:    "udp_only",
	}); err != nil {
		t.Fatalf("publish dial1 visitor info error = %v, want nil", err)
	}

	var dial1Client wire.NatHoleClient
	waitCtx, cancelWait := context.WithTimeout(ctx, 2*time.Second)
	defer cancelWait()
	if err := visitorSess.waitJSON(waitCtx, visitorSess.attemptTopic(dial1, "info/client"), &dial1Client); err != nil {
		t.Fatalf("wait dial1 client info error = %v, want nil", err)
	}

	pub2Ctx, cancelPub2 := context.WithTimeout(ctx, 2*time.Second)
	defer cancelPub2()
	if err := visitorSess.publishJSON(pub2Ctx, visitorSess.attemptTopic(dial2, "info/visitor"), &wire.NatHoleVisitor{
		TransactionID: dial2,
		ProxyName:     "proxy",
		Protocol:      "quic",
		P2PNetwork:    "udp_only",
	}); err != nil {
		t.Fatalf("publish dial2 visitor info error = %v, want nil", err)
	}

	var gotDrop bool
	for !gotDrop {
		select {
		case at := <-attemptCh:
			if at.DialID == dial2 && at.Err == ErrTooManyAttempts {
				gotDrop = true
				break
			}
		case err := <-clientErrCh:
			t.Fatalf("ServeClient returned early: %v", err)
		case <-ctx.Done():
			t.Fatalf("timed out waiting for ErrTooManyAttempts")
		}
	}

	clientRunCancel()
	select {
	case err := <-clientErrCh:
		if err != nil {
			t.Fatalf("ServeClient error = %v, want nil", err)
		}
	case <-ctx.Done():
		t.Fatalf("timed out waiting for ServeClient to stop")
	}
}

func keys(m map[string]ClientAttempt) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	return out
}
