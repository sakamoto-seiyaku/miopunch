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

	"github.com/miopunch/miopunch/xtcp/control"
	"github.com/miopunch/miopunch/xtcp/msg"
	"github.com/miopunch/miopunch/xtcp/transport"
)

type controlSession struct {
	rwc        io.ReadWriteCloser
	dispatcher *msg.Dispatcher
	xport      transport.MessageTransporter
}

func dialHello(ctx context.Context, coordAddr string, proto control.Protocol, hello *msg.PeerHello, timeout time.Duration) (*controlSession, error) {
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

	disp := msg.NewDispatcher(rwc)
	xport := transport.NewMessageTransporter(disp)

	disp.RegisterHandler(&msg.NatHoleResp{}, func(m msg.Message) {
		in := m.(*msg.NatHoleResp)
		_ = xport.DispatchWithType(in, msg.TypeNameNatHoleResp, in.TransactionID)
	})

	helloRespCh := make(chan *msg.PeerHelloResp, 1)
	disp.RegisterHandler(&msg.PeerHelloResp{}, func(m msg.Message) {
		select {
		case helloRespCh <- m.(*msg.PeerHelloResp):
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
