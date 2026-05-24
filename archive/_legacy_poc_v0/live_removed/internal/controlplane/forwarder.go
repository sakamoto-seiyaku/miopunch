package controlplane

import (
	"errors"
	"fmt"
	"sync"
	"sync/atomic"
)

const (
	defaultForwardQueueMax = 1024
)

type SendFunc func(ciphertext []byte) error

type DeliverFunc func(srcNeighborID string, msg Message) error

type ForwarderConfig struct {
	NetSecret  []byte
	SelfPeerID string

	Seen      *SeenCache
	Neighbors map[string]SendFunc
	Deliver   DeliverFunc

	ForwardQueueMax int
}

type ForwarderStats struct {
	MeshForwardDrops int64

	DecryptDrops  int64
	DecodeDrops   int64
	HopLimitDrops int64
	DedupDrops    int64
	DeliverDrops  int64
	SendErrors    int64
}

type Forwarder struct {
	netSecret  []byte
	selfPeerID string

	seen      *SeenCache
	neighbors map[string]SendFunc
	deliver   DeliverFunc

	// forwardCh implements the POC v0 bounded forward queue (spec: forward_queue_max=1024).
	// This intentionally exceeds the "0 or 1" channel buffer heuristic: the spec requires a
	// bounded queue large enough to absorb short bursts without blocking UDP receive loops.
	// When full, new forwarding work is dropped and counted in MeshForwardDrops.
	forwardCh chan forwardWork
	closeC    chan struct{}
	wg        sync.WaitGroup
	closed    atomic.Bool

	meshForwardDrops atomic.Int64

	decryptDrops  atomic.Int64
	decodeDrops   atomic.Int64
	hopLimitDrops atomic.Int64
	dedupDrops    atomic.Int64
	deliverDrops  atomic.Int64
	sendErrors    atomic.Int64
}

type forwardWork struct {
	srcNeighborID string
	ciphertext    []byte
}

func NewForwarder(cfg ForwarderConfig) (*Forwarder, error) {
	if len(cfg.NetSecret) == 0 {
		return nil, errors.New("net_secret is required")
	}
	if cfg.SelfPeerID == "" {
		return nil, errors.New("self_peer_id is required")
	}

	seen := cfg.Seen
	if seen == nil {
		seen = NewSeenCache(0, 0, nil)
	}

	forwardQueueMax := cfg.ForwardQueueMax
	if forwardQueueMax <= 0 {
		forwardQueueMax = defaultForwardQueueMax
	}

	neighbors := make(map[string]SendFunc, len(cfg.Neighbors))
	for k, v := range cfg.Neighbors {
		neighbors[k] = v
	}

	f := &Forwarder{
		netSecret:  cfg.NetSecret,
		selfPeerID: cfg.SelfPeerID,
		seen:       seen,
		neighbors:  neighbors,
		deliver:    cfg.Deliver,
		forwardCh:  make(chan forwardWork, forwardQueueMax),
		closeC:     make(chan struct{}),
	}
	f.wg.Add(1)
	go func() {
		defer f.wg.Done()
		f.run()
	}()
	return f, nil
}

func (f *Forwarder) Close() error {
	if f == nil {
		return nil
	}
	if !f.closed.CompareAndSwap(false, true) {
		return nil
	}
	close(f.closeC)
	f.wg.Wait()
	return nil
}

func (f *Forwarder) HandleInbound(srcNeighborID string, ciphertext []byte) {
	if f == nil || f.closed.Load() {
		return
	}

	pt, err := OpenGroupV0(f.netSecret, ciphertext)
	if err != nil {
		f.decryptDrops.Add(1)
		return
	}
	msg, err := UnmarshalMessage(pt)
	if err != nil {
		f.decodeDrops.Add(1)
		return
	}
	if msg.ProtoVersion != ProtoVersionV0 {
		f.decodeDrops.Add(1)
		return
	}

	canonicalMsgID, err := CanonicalizeMsgID(msg.Route.MsgID)
	if err != nil || canonicalMsgID != msg.Route.MsgID {
		f.decodeDrops.Add(1)
		return
	}

	if msg.Route.HopLimit < 0 || msg.Route.HopLimit > HopLimitMax {
		f.hopLimitDrops.Add(1)
		return
	}

	seenBefore := f.seen.SeenBefore(canonicalMsgID)
	if seenBefore {
		// Dedup applies strictly to forwarding and best-effort delivery.
		//
		// For dst=self RPC requests, duplicate delivery MUST reach the upper layer
		// so it can re-send the cached final response without re-applying side effects.
		if msg.Route.DstPeerID != f.selfPeerID || !IsRPCRequest(msg.Signed.Kind) {
			f.dedupDrops.Add(1)
			return
		}
	}

	if msg.Route.DstPeerID == f.selfPeerID {
		if f.deliver == nil {
			return
		}
		if err := f.deliver(srcNeighborID, msg); err != nil {
			f.deliverDrops.Add(1)
		}
		return
	}

	if msg.Route.HopLimit == 0 {
		return
	}

	msg.Route.HopLimit--
	pt2, err := MarshalMessage(msg)
	if err != nil {
		f.decodeDrops.Add(1)
		return
	}
	ct2, err := SealGroupV0(f.netSecret, pt2)
	if err != nil {
		f.decryptDrops.Add(1)
		return
	}

	work := forwardWork{
		srcNeighborID: srcNeighborID,
		ciphertext:    ct2,
	}

	select {
	case f.forwardCh <- work:
	default:
		f.meshForwardDrops.Add(1)
	}
}

func (f *Forwarder) Stats() ForwarderStats {
	if f == nil {
		return ForwarderStats{}
	}
	return ForwarderStats{
		MeshForwardDrops: f.meshForwardDrops.Load(),
		DecryptDrops:     f.decryptDrops.Load(),
		DecodeDrops:      f.decodeDrops.Load(),
		HopLimitDrops:    f.hopLimitDrops.Load(),
		DedupDrops:       f.dedupDrops.Load(),
		DeliverDrops:     f.deliverDrops.Load(),
		SendErrors:       f.sendErrors.Load(),
	}
}

func (f *Forwarder) run() {
	for {
		select {
		case <-f.closeC:
			return
		case work := <-f.forwardCh:
			f.sendToNeighbors(work)
		}
	}
}

func (f *Forwarder) sendToNeighbors(work forwardWork) {
	for neighborID, send := range f.neighbors {
		if neighborID == work.srcNeighborID {
			continue
		}
		if send == nil {
			continue
		}
		if err := send(work.ciphertext); err != nil {
			f.sendErrors.Add(1)
		}
	}
}

func (f *Forwarder) String() string {
	if f == nil {
		return "<nil>"
	}
	return fmt.Sprintf("Forwarder{self=%s}", f.selfPeerID)
}
