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

package peer

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/control"
	"github.com/miopunch/miopunch/internal/wire"
)

type controlSession struct {
	rwc        io.ReadWriteCloser
	dispatcher *wire.Dispatcher
	xport      wire.MessageTransporter
}

func dialHello(ctx context.Context, coordAddr string, proto control.Protocol, hello *wire.PeerHello, timeout time.Duration) (*controlSession, error) {
	if strings.TrimSpace(coordAddr) == "" {
		return nil, errors.New("coord addr is required")
	}
	if proto == "" {
		proto = control.ProtoTCP
	}
	if timeout == 0 {
		timeout = 5 * time.Second
	}

	rwc, err := control.Dial(ctx, coordAddr, proto)
	if err != nil {
		return nil, err
	}

	disp := wire.NewDispatcher(rwc)
	xport := wire.NewMessageTransporter(disp)

	disp.RegisterHandler(&wire.NatHoleResp{}, func(m wire.Message) {
		in := m.(*wire.NatHoleResp)
		_ = xport.DispatchWithType(in, wire.TypeNameNatHoleResp, in.TransactionID)
	})

	helloRespCh := make(chan *wire.PeerHelloResp, 1)
	disp.RegisterHandler(&wire.PeerHelloResp{}, func(m wire.Message) {
		select {
		case helloRespCh <- m.(*wire.PeerHelloResp):
		default:
		}
	})

	disp.Run()

	if err := disp.Send(hello); err != nil {
		_ = rwc.Close()
		return nil, err
	}

	select {
	case <-ctx.Done():
		_ = rwc.Close()
		return nil, ctx.Err()
	case <-disp.Done():
		_ = rwc.Close()
		return nil, io.EOF
	case <-time.After(timeout):
		_ = rwc.Close()
		return nil, fmt.Errorf("hello timeout")
	case resp := <-helloRespCh:
		if strings.TrimSpace(resp.Error) != "" {
			_ = rwc.Close()
			return nil, fmt.Errorf("hello rejected: %s", resp.Error)
		}
	}

	return &controlSession{
		rwc:        rwc,
		dispatcher: disp,
		xport:      xport,
	}, nil
}

func writeFrame(w io.Writer, payload []byte) error {
	var hdr [4]byte
	binary.BigEndian.PutUint32(hdr[:], uint32(len(payload)))
	if _, err := w.Write(hdr[:]); err != nil {
		return err
	}
	_, err := w.Write(payload)
	return err
}

func readFrame(r io.Reader, max int) ([]byte, error) {
	var hdr [4]byte
	if _, err := io.ReadFull(r, hdr[:]); err != nil {
		return nil, err
	}
	n := int(binary.BigEndian.Uint32(hdr[:]))
	if n < 0 || n > max {
		return nil, fmt.Errorf("frame too large: %d", n)
	}
	buf := make([]byte, n)
	if _, err := io.ReadFull(r, buf); err != nil {
		return nil, err
	}
	return buf, nil
}
