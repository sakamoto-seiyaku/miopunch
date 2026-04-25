// Copyright 2023 The frp Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

package wire

import (
	"io"
	"reflect"
	"sync"
)

func AsyncHandler(f func(Message)) func(Message) {
	return func(m Message) {
		go f(m)
	}
}

// Dispatcher is used to send messages to io.ReadWriter or register handlers for messages read from it.
type Dispatcher struct {
	rw io.ReadWriter

	sendCh chan Message
	doneCh chan struct{}

	handlersMu     sync.RWMutex
	msgHandlers    map[reflect.Type]func(Message)
	defaultHandler func(Message)

	doneOnce sync.Once
	errMu    sync.RWMutex
	err      error
}

func NewDispatcher(rw io.ReadWriter) *Dispatcher {
	return &Dispatcher{
		rw:          rw,
		sendCh:      make(chan Message, 100),
		doneCh:      make(chan struct{}),
		msgHandlers: make(map[reflect.Type]func(Message)),
	}
}

// Run will return when io.EOF or some error occurs.
func (d *Dispatcher) Run() {
	go d.sendLoop()
	go d.readLoop()
}

func (d *Dispatcher) sendLoop() {
	for {
		select {
		case <-d.doneCh:
			return
		case m := <-d.sendCh:
			if err := WriteMsg(d.rw, m); err != nil {
				d.closeWithError(err)
				return
			}
		}
	}
}

func (d *Dispatcher) readLoop() {
	for {
		m, err := ReadMsg(d.rw)
		if err != nil {
			d.closeWithError(err)
			return
		}

		handler := d.handlerFor(m)
		if handler != nil {
			handler(m)
		}
	}
}

func (d *Dispatcher) Send(m Message) error {
	select {
	case <-d.doneCh:
		if err := d.Err(); err != nil {
			return err
		}
		return io.EOF
	default:
	}

	select {
	case <-d.doneCh:
		if err := d.Err(); err != nil {
			return err
		}
		return io.EOF
	case d.sendCh <- m:
		return nil
	}
}

func (d *Dispatcher) RegisterHandler(msg Message, handler func(Message)) {
	d.handlersMu.Lock()
	defer d.handlersMu.Unlock()
	d.msgHandlers[reflect.TypeOf(msg)] = handler
}

func (d *Dispatcher) RegisterDefaultHandler(handler func(Message)) {
	d.handlersMu.Lock()
	defer d.handlersMu.Unlock()
	d.defaultHandler = handler
}

func (d *Dispatcher) Done() chan struct{} {
	return d.doneCh
}

func (d *Dispatcher) Err() error {
	d.errMu.RLock()
	defer d.errMu.RUnlock()
	return d.err
}

func (d *Dispatcher) handlerFor(m Message) func(Message) {
	d.handlersMu.RLock()
	defer d.handlersMu.RUnlock()
	if handler, ok := d.msgHandlers[reflect.TypeOf(m)]; ok {
		return handler
	}
	return d.defaultHandler
}

func (d *Dispatcher) closeWithError(err error) {
	d.doneOnce.Do(func() {
		d.errMu.Lock()
		d.err = err
		d.errMu.Unlock()
		close(d.doneCh)
	})
}
