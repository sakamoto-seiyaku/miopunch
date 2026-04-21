package controlplane

import (
	"bytes"
	"crypto/ed25519"
	"encoding/json"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestForwarder_ThreeNodeForwarding_H3_Dedup_SourceExcluded(t *testing.T) {
	netSecret := []byte("0123456789abcdef0123456789abcdef")

	peerA := testBase32ID("peer-A")
	peerB := testBase32ID("peer-B")
	peerC := testBase32ID("peer-C")

	deliveredC := make(chan Message, 1)
	fwdC, err := NewForwarder(ForwarderConfig{
		NetSecret:  netSecret,
		SelfPeerID: peerC,
		Deliver: func(_ string, msg Message) error {
			select {
			case deliveredC <- msg:
			default:
			}
			return nil
		},
	})
	if err != nil {
		t.Fatalf("NewForwarder(C) error = %v", err)
	}
	t.Cleanup(func() { _ = fwdC.Close() })

	var sentToA atomic.Int64
	sendToA := func([]byte) error {
		sentToA.Add(1)
		return nil
	}
	sendToC := func(ciphertext []byte) error {
		fwdC.HandleInbound(peerB, ciphertext)
		return nil
	}

	fwdB, err := NewForwarder(ForwarderConfig{
		NetSecret:  netSecret,
		SelfPeerID: peerB,
		Neighbors: map[string]SendFunc{
			peerA: sendToA,
			peerC: sendToC,
		},
	})
	if err != nil {
		t.Fatalf("NewForwarder(B) error = %v", err)
	}
	t.Cleanup(func() { _ = fwdB.Close() })

	body, err := json.Marshal(struct {
		Msg string `json:"msg"`
	}{Msg: "hi"})
	if err != nil {
		t.Fatalf("json.Marshal(body) error = %v", err)
	}

	seed := []byte("0123456789abcdef0123456789abcdef")
	priv := ed25519.NewKeyFromSeed(seed)

	req := Message{
		ProtoVersion: ProtoVersionV0,
		Route: Route{
			DstPeerID:       peerC,
			MsgID:           testBase32ID("msg-forward-1"),
			HopLimit:        HopLimitMax,
			CreatedAtUnixMs: 1,
		},
		Signed: Signed{
			SenderPeerID: peerA,
			Kind:         "smoke_echo_req",
			Body:         body,
		},
	}
	if err := SignV0(priv, &req); err != nil {
		t.Fatalf("SignV0() error = %v", err)
	}

	pt, err := MarshalMessage(req)
	if err != nil {
		t.Fatalf("MarshalMessage() error = %v", err)
	}
	ct, err := SealGroupV0(netSecret, pt)
	if err != nil {
		t.Fatalf("SealGroupV0() error = %v", err)
	}

	fwdB.HandleInbound(peerA, ct)

	select {
	case got := <-deliveredC:
		if got.Route.DstPeerID != peerC {
			t.Fatalf("delivered dst_peer_id = %q, want %q", got.Route.DstPeerID, peerC)
		}
		if got.Route.HopLimit != HopLimitMax-1 {
			t.Fatalf("delivered hop_limit = %d, want %d", got.Route.HopLimit, HopLimitMax-1)
		}
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for delivery at C")
	}

	if got := sentToA.Load(); got != 0 {
		t.Fatalf("forwarder B forwarded back to A: sends=%d, want 0", got)
	}

	// Dedup: sending the same ciphertext again should not deliver again.
	fwdB.HandleInbound(peerA, ct)

	select {
	case <-deliveredC:
		t.Fatalf("unexpected duplicate delivery at C")
	case <-time.After(200 * time.Millisecond):
	}

	if got := fwdB.Stats().DedupDrops; got != 1 {
		t.Fatalf("B DedupDrops = %d, want %d", got, 1)
	}
}

func TestForwarder_ForwardQueueDropsWhenFull(t *testing.T) {
	netSecret := []byte("0123456789abcdef0123456789abcdef")

	peerA := testBase32ID("peer-A")
	peerB := testBase32ID("peer-B")
	peerC := testBase32ID("peer-C")

	sendStarted := make(chan struct{}, 1)
	unblock := make(chan struct{})
	blockingSend := func([]byte) error {
		select {
		case sendStarted <- struct{}{}:
		default:
		}
		<-unblock
		return nil
	}

	fwdB, err := NewForwarder(ForwarderConfig{
		NetSecret:       netSecret,
		SelfPeerID:      peerB,
		ForwardQueueMax: 1,
		Neighbors: map[string]SendFunc{
			peerC: blockingSend,
		},
	})
	if err != nil {
		t.Fatalf("NewForwarder(B) error = %v", err)
	}
	t.Cleanup(func() {
		close(unblock)
		_ = fwdB.Close()
	})

	body, err := json.Marshal(struct {
		N int `json:"n"`
	}{N: 1})
	if err != nil {
		t.Fatalf("json.Marshal(body) error = %v", err)
	}

	seed := []byte("0123456789abcdef0123456789abcdef")
	priv := ed25519.NewKeyFromSeed(seed)

	mk := func(msgID string) []byte {
		m := Message{
			ProtoVersion: ProtoVersionV0,
			Route: Route{
				DstPeerID:       peerC,
				MsgID:           msgID,
				HopLimit:        1,
				CreatedAtUnixMs: 1,
			},
			Signed: Signed{
				SenderPeerID: peerA,
				Kind:         "smoke_echo_req",
				Body:         body,
			},
		}
		if err := SignV0(priv, &m); err != nil {
			t.Fatalf("SignV0() error = %v", err)
		}
		pt, err := MarshalMessage(m)
		if err != nil {
			t.Fatalf("MarshalMessage() error = %v", err)
		}
		ct, err := SealGroupV0(netSecret, pt)
		if err != nil {
			t.Fatalf("SealGroupV0() error = %v", err)
		}
		return ct
	}

	fwdB.HandleInbound(peerA, mk(testBase32ID("msg-q-1")))

	select {
	case <-sendStarted:
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for send to start")
	}

	fwdB.HandleInbound(peerA, mk(testBase32ID("msg-q-2"))) // fills queue (cap=1)
	fwdB.HandleInbound(peerA, mk(testBase32ID("msg-q-3"))) // should drop

	if got := fwdB.Stats().MeshForwardDrops; got != 1 {
		t.Fatalf("MeshForwardDrops = %d, want %d", got, 1)
	}
}

func TestForwarder_StrictMsgIDRejectsNonCanonical(t *testing.T) {
	netSecret := []byte("0123456789abcdef0123456789abcdef")

	peerA := testBase32ID("peer-A")
	peerB := testBase32ID("peer-B")

	deliverCalled := make(chan struct{}, 1)
	fwdB, err := NewForwarder(ForwarderConfig{
		NetSecret:  netSecret,
		SelfPeerID: peerB,
		Deliver: func(_ string, _ Message) error {
			select {
			case deliverCalled <- struct{}{}:
			default:
			}
			return nil
		},
	})
	if err != nil {
		t.Fatalf("NewForwarder(B) error = %v", err)
	}
	t.Cleanup(func() { _ = fwdB.Close() })

	body, err := json.Marshal(struct {
		N int `json:"n"`
	}{N: 1})
	if err != nil {
		t.Fatalf("json.Marshal(body) error = %v", err)
	}

	seed := []byte("0123456789abcdef0123456789abcdef")
	priv := ed25519.NewKeyFromSeed(seed)

	msgID := testBase32ID("msg-non-canonical")
	msgID = strings.ToLower(msgID)
	m := Message{
		ProtoVersion: ProtoVersionV0,
		Route: Route{
			DstPeerID:       peerB,
			MsgID:           msgID,
			HopLimit:        HopLimitMax,
			CreatedAtUnixMs: 1,
		},
		Signed: Signed{
			SenderPeerID: peerA,
			Kind:         "smoke_echo_req",
			Body:         body,
		},
	}
	if err := SignV0(priv, &m); err != nil {
		t.Fatalf("SignV0() error = %v", err)
	}

	pt, err := MarshalMessage(m)
	if err != nil {
		t.Fatalf("MarshalMessage() error = %v", err)
	}
	ct, err := SealGroupV0(netSecret, pt)
	if err != nil {
		t.Fatalf("SealGroupV0() error = %v", err)
	}

	fwdB.HandleInbound(peerA, ct)

	select {
	case <-deliverCalled:
		t.Fatalf("deliver unexpectedly called for non-canonical msg_id")
	case <-time.After(200 * time.Millisecond):
	}
	if got := fwdB.Stats().DecodeDrops; got != 1 {
		t.Fatalf("DecodeDrops = %d, want %d", got, 1)
	}
}

func TestForwarder_SelfRPCRequestDuplicateIsDelivered(t *testing.T) {
	netSecret := []byte("0123456789abcdef0123456789abcdef")

	peerA := testBase32ID("peer-A")
	peerB := testBase32ID("peer-B")

	delivered := make(chan Message, 2)
	fwdB, err := NewForwarder(ForwarderConfig{
		NetSecret:  netSecret,
		SelfPeerID: peerB,
		Deliver: func(_ string, msg Message) error {
			delivered <- msg
			return nil
		},
	})
	if err != nil {
		t.Fatalf("NewForwarder(B) error = %v", err)
	}
	t.Cleanup(func() { _ = fwdB.Close() })

	body, err := json.Marshal(struct {
		N int `json:"n"`
	}{N: 1})
	if err != nil {
		t.Fatalf("json.Marshal(body) error = %v", err)
	}

	seed := []byte("0123456789abcdef0123456789abcdef")
	priv := ed25519.NewKeyFromSeed(seed)

	req := Message{
		ProtoVersion: ProtoVersionV0,
		Route: Route{
			DstPeerID:       peerB,
			MsgID:           testBase32ID("req-1"),
			HopLimit:        0,
			CreatedAtUnixMs: 1,
			ExpiresAtUnixMs: 2,
		},
		Signed: Signed{
			SenderPeerID: peerA,
			Kind:         "echo_request",
			Body:         body,
		},
	}
	if err := SignV0(priv, &req); err != nil {
		t.Fatalf("SignV0() error = %v", err)
	}

	pt, err := MarshalMessage(req)
	if err != nil {
		t.Fatalf("MarshalMessage() error = %v", err)
	}
	ct, err := SealGroupV0(netSecret, pt)
	if err != nil {
		t.Fatalf("SealGroupV0() error = %v", err)
	}

	fwdB.HandleInbound(peerA, ct)
	fwdB.HandleInbound(peerA, ct)

	for i := 0; i < 2; i++ {
		select {
		case got := <-delivered:
			if got.Route.MsgID != req.Route.MsgID {
				t.Fatalf("delivered msg_id = %q, want %q", got.Route.MsgID, req.Route.MsgID)
			}
		case <-time.After(2 * time.Second):
			t.Fatalf("timed out waiting for delivery %d/2", i+1)
		}
	}

	if got := fwdB.Stats().DedupDrops; got != 0 {
		t.Fatalf("DedupDrops = %d, want %d", got, 0)
	}
}

func TestForwarder_SelfRPCRequestDuplicateResendsCachedResponse(t *testing.T) {
	netSecret := []byte("0123456789abcdef0123456789abcdef")

	peerA := testBase32ID("peer-A")
	peerB := testBase32ID("peer-B")

	seedA := []byte("0123456789abcdef0123456789abcdef")
	privA := ed25519.NewKeyFromSeed(seedA)

	seedB := []byte("fedcba9876543210fedcba9876543210")
	privB := ed25519.NewKeyFromSeed(seedB)

	handled := NewHandledRequestsCache(16, 1*time.Hour, func() time.Time { return time.Unix(0, 0).UTC() })

	var sideEffects atomic.Int64
	sentToA := make(chan []byte, 2)

	fwdB, err := NewForwarder(ForwarderConfig{
		NetSecret:  netSecret,
		SelfPeerID: peerB,
		Deliver: func(_ string, msg Message) error {
			if msg.Signed.Kind != "echo_request" {
				return nil
			}

			respCT, _, err := handled.Handle(msg, func() ([]byte, error) {
				sideEffects.Add(1)

				body, err := json.Marshal(struct {
					OK bool `json:"ok"`
				}{OK: true})
				if err != nil {
					return nil, err
				}

				resp := Message{
					ProtoVersion: ProtoVersionV0,
					Route: Route{
						DstPeerID:       msg.Signed.SenderPeerID,
						MsgID:           testBase32ID("resp-1"),
						HopLimit:        0,
						CreatedAtUnixMs: 1,
					},
					Signed: Signed{
						SenderPeerID: peerB,
						Kind:         "echo_response",
						InReplyTo:    msg.Route.MsgID,
						Body:         body,
					},
				}
				if err := SignV0(privB, &resp); err != nil {
					return nil, err
				}
				pt, err := MarshalMessage(resp)
				if err != nil {
					return nil, err
				}
				return SealGroupV0(netSecret, pt)
			})
			if err != nil {
				return err
			}

			sentToA <- respCT
			return nil
		},
	})
	if err != nil {
		t.Fatalf("NewForwarder(B) error = %v", err)
	}
	t.Cleanup(func() { _ = fwdB.Close() })

	body, err := json.Marshal(struct {
		Msg string `json:"msg"`
	}{Msg: "hi"})
	if err != nil {
		t.Fatalf("json.Marshal(body) error = %v", err)
	}

	req := Message{
		ProtoVersion: ProtoVersionV0,
		Route: Route{
			DstPeerID:       peerB,
			MsgID:           testBase32ID("req-1"),
			HopLimit:        0,
			CreatedAtUnixMs: 1,
			ExpiresAtUnixMs: 2,
		},
		Signed: Signed{
			SenderPeerID: peerA,
			Kind:         "echo_request",
			Body:         body,
		},
	}
	if err := SignV0(privA, &req); err != nil {
		t.Fatalf("SignV0() error = %v", err)
	}

	pt, err := MarshalMessage(req)
	if err != nil {
		t.Fatalf("MarshalMessage() error = %v", err)
	}
	ct, err := SealGroupV0(netSecret, pt)
	if err != nil {
		t.Fatalf("SealGroupV0() error = %v", err)
	}

	fwdB.HandleInbound(peerA, ct)
	fwdB.HandleInbound(peerA, ct)

	var ct1, ct2 []byte
	select {
	case ct1 = <-sentToA:
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for response send 1/2")
	}
	select {
	case ct2 = <-sentToA:
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for response send 2/2")
	}

	if !bytes.Equal(ct1, ct2) {
		t.Fatalf("cached response ciphertext mismatch: len1=%d len2=%d", len(ct1), len(ct2))
	}
	if got := sideEffects.Load(); got != 1 {
		t.Fatalf("side effects applied %d times, want %d", got, 1)
	}
}
