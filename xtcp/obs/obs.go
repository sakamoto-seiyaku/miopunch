// Copyright 2026 The miopunch Authors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

package obs

import (
	"encoding/json"
	"io"
	"sync"
	"time"
)

type Stage string

const (
	StageDiscovery  Stage = "discovery"
	StageSignaling  Stage = "signaling"
	StagePunching   Stage = "punching"
	StageConfirm    Stage = "confirm"
	StageTransport  Stage = "transport"
	StageSupervisor Stage = "supervisor"
)

type Kind string

const (
	KindStart Kind = "start"
	KindOK    Kind = "ok"
	KindFail  Kind = "fail"
	KindInfo  Kind = "info"
)

type Event struct {
	TS     string `json:"ts"`
	UnixMs int64  `json:"unix_ms"`

	Role  string `json:"role,omitempty"`
	Stage Stage  `json:"stage,omitempty"`
	Kind  Kind   `json:"kind,omitempty"`

	SID          string `json:"sid,omitempty"`
	Transaction  string `json:"tx,omitempty"`
	ControlProto string `json:"control_proto,omitempty"`
	DataProto    string `json:"data_proto,omitempty"`

	Msg string         `json:"msg,omitempty"`
	Err string         `json:"err,omitempty"`
	KVs map[string]any `json:"kvs,omitempty"`
}

type Emitter struct {
	mu    sync.Mutex
	out   io.Writer
	role  string
	clock func() time.Time
}

func NewEmitter(out io.Writer, role string) *Emitter {
	return &Emitter{
		out:  out,
		role: role,
		clock: func() time.Time {
			return time.Now().UTC()
		},
	}
}

func (e *Emitter) Emit(ev Event) {
	now := e.clock()
	if ev.TS == "" {
		ev.TS = now.Format(time.RFC3339Nano)
	}
	if ev.UnixMs == 0 {
		ev.UnixMs = now.UnixMilli()
	}
	if ev.Role == "" {
		ev.Role = e.role
	}

	e.mu.Lock()
	defer e.mu.Unlock()
	enc := json.NewEncoder(e.out)
	_ = enc.Encode(ev)
}

func (e *Emitter) Start(stage Stage, msg string, kvs map[string]any) {
	e.Emit(Event{Stage: stage, Kind: KindStart, Msg: msg, KVs: kvs})
}

func (e *Emitter) OK(stage Stage, msg string, kvs map[string]any) {
	e.Emit(Event{Stage: stage, Kind: KindOK, Msg: msg, KVs: kvs})
}

func (e *Emitter) Fail(stage Stage, err error, msg string, kvs map[string]any) {
	ev := Event{Stage: stage, Kind: KindFail, Msg: msg, KVs: kvs}
	if err != nil {
		ev.Err = err.Error()
	}
	e.Emit(ev)
}
