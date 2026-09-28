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

package punching

import (
	"github.com/miopunch/miopunch/internal/punchwire"
	"github.com/miopunch/miopunch/internal/wire"
)

var (
	PunchTagV1        = punchwire.PunchTagV1
	ErrNotPunchPacket = punchwire.ErrNotPunchPacket
)

func HasPunchTag(b []byte) bool { return punchwire.HasPunchTag(b) }

func EncodeMessage(m wire.Message, key []byte) ([]byte, error) {
	return punchwire.EncodeMessage(m, key)
}

func DecodeMessageInto(data, key []byte, m wire.Message) error {
	return punchwire.DecodeMessageInto(data, key, m)
}
