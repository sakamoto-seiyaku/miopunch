package bbr

import (
	"github.com/apernet/quic-go/congestion"
)

// NOTE: Derived from Hysteria2's BBR sender implementation.
// Source: apernet/hysteria (tag: app/v2.7.1), core/internal/congestion/bbr/packet_number_indexed_queue.go

// packetNumberIndexedQueue is a queue of mostly continuous numbered entries
// which supports the following operations:
// - adding elements to the end of the queue, or at some point past the end
// - removing elements in any order
// - retrieving elements
// If all elements are inserted in order, all of the operations above are
// amortized O(1) time.
//
// Internally, the data structure is a deque where each element is marked as
// present or not. The deque starts at the lowest present index. Whenever an
// element is removed, it's marked as not present, and the front of the deque is
// cleared of elements that are not present.
//
// The tail of the queue is not cleared due to the assumption of entries being
// inserted in order, though removing all elements of the queue will return it
// to its initial state.
//
// Note that this data structure is inherently hazardous, since an addition of
// just two entries will cause it to consume all of the memory available.
// Because of that, it is not a general-purpose container and should not be used
// as one.

type entryWrapper[T any] struct {
	present bool
	entry   T
}

type packetNumberIndexedQueue[T any] struct {
	entries                RingBuffer[entryWrapper[T]]
	numberOfPresentEntries int
	firstPacket            congestion.PacketNumber
}

func newPacketNumberIndexedQueue[T any](size int) *packetNumberIndexedQueue[T] {
	q := &packetNumberIndexedQueue[T]{
		firstPacket: invalidPacketNumber,
	}
	q.entries.Init(size)
	return q
}

// Emplace inserts data associated with packetNumber into (or past) the end of the queue,
// filling up the missing intermediate entries as necessary.
// Returns true if the element has been inserted successfully, false if it was already in
// the queue or inserted out of order.
func (p *packetNumberIndexedQueue[T]) Emplace(packetNumber congestion.PacketNumber, entry *T) bool {
	if packetNumber == invalidPacketNumber || entry == nil {
		return false
	}

	if p.IsEmpty() {
		p.entries.PushBack(entryWrapper[T]{present: true, entry: *entry})
		p.numberOfPresentEntries = 1
		p.firstPacket = packetNumber
		return true
	}

	// Do not allow insertion out-of-order.
	if packetNumber <= p.LastPacket() {
		return false
	}

	// Handle potentially missing elements.
	offset := int(packetNumber - p.FirstPacket())
	if gap := offset - p.entries.Len(); gap > 0 {
		for i := 0; i < gap; i++ {
			p.entries.PushBack(entryWrapper[T]{})
		}
	}

	p.entries.PushBack(entryWrapper[T]{present: true, entry: *entry})
	p.numberOfPresentEntries++
	return true
}

// GetEntry retrieves the entry associated with the packet number.
// Returns a pointer to the entry in case of success, or nil if the entry does not exist.
func (p *packetNumberIndexedQueue[T]) GetEntry(packetNumber congestion.PacketNumber) *T {
	ew := p.getEntryWraper(packetNumber)
	if ew == nil {
		return nil
	}
	return &ew.entry
}

// Remove is like GetEntry, but if an entry is present in the queue, it also calls f(entry)
// before removing it.
func (p *packetNumberIndexedQueue[T]) Remove(packetNumber congestion.PacketNumber, f func(T)) bool {
	ew := p.getEntryWraper(packetNumber)
	if ew == nil {
		return false
	}
	if f != nil {
		f(ew.entry)
	}
	ew.present = false
	p.numberOfPresentEntries--

	if packetNumber == p.FirstPacket() {
		p.clearup()
	}
	return true
}

// RemoveUpTo removes entries up to, but not including, packetNumber.
// Unused slots in the front are also removed, which means when the function returns,
// FirstPacket() can be larger than packetNumber.
func (p *packetNumberIndexedQueue[T]) RemoveUpTo(packetNumber congestion.PacketNumber) {
	for !p.entries.Empty() &&
		p.firstPacket != invalidPacketNumber &&
		p.firstPacket < packetNumber {
		if p.entries.Front().present {
			p.numberOfPresentEntries--
		}
		p.entries.PopFront()
		p.firstPacket++
	}
	p.clearup()
}

// IsEmpty returns true if queue is empty.
func (p *packetNumberIndexedQueue[T]) IsEmpty() bool {
	return p.numberOfPresentEntries == 0
}

// NumberOfPresentEntries returns the number of entries in the queue.
func (p *packetNumberIndexedQueue[T]) NumberOfPresentEntries() int {
	return p.numberOfPresentEntries
}

// EntrySlotsUsed returns the number of entries allocated in the underlying deque.
// This is proportional to the memory usage of the queue.
func (p *packetNumberIndexedQueue[T]) EntrySlotsUsed() int {
	return p.entries.Len()
}

// FirstPacket returns packet number of the first entry in the queue.
func (p *packetNumberIndexedQueue[T]) FirstPacket() congestion.PacketNumber {
	return p.firstPacket
}

// LastPacket returns packet number of the last entry ever inserted in the queue.
// Note that the entry in question may have already been removed.
// Returns invalidPacketNumber if the queue is empty.
func (p *packetNumberIndexedQueue[T]) LastPacket() congestion.PacketNumber {
	if p.IsEmpty() {
		return invalidPacketNumber
	}
	return p.firstPacket + congestion.PacketNumber(p.entries.Len()-1)
}

func (p *packetNumberIndexedQueue[T]) clearup() {
	for !p.entries.Empty() && !p.entries.Front().present {
		p.entries.PopFront()
		p.firstPacket++
	}
	if p.entries.Empty() {
		p.firstPacket = invalidPacketNumber
	}
}

func (p *packetNumberIndexedQueue[T]) getEntryWraper(packetNumber congestion.PacketNumber) *entryWrapper[T] {
	if packetNumber == invalidPacketNumber || p.IsEmpty() || packetNumber < p.firstPacket {
		return nil
	}

	offset := int(packetNumber - p.firstPacket)
	if offset >= p.entries.Len() {
		return nil
	}

	ew := p.entries.Offset(offset)
	if ew == nil || !ew.present {
		return nil
	}
	return ew
}
