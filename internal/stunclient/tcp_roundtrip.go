package stunclient

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"net"
	"time"

	"github.com/pion/stun/v2"
)

const maxTCPMessageSize = 2048

func RoundTripTCP(ctx context.Context, dialer *net.Dialer, addr string) (mappedAddr string, rtt time.Duration, err error) {
	if dialer == nil {
		dialer = &net.Dialer{}
	}

	conn, err := dialer.DialContext(ctx, "tcp", addr)
	if err != nil {
		return "", 0, err
	}
	defer conn.Close()

	if deadline, ok := ctx.Deadline(); ok {
		_ = conn.SetDeadline(deadline)
	}

	req, err := stun.Build(stun.TransactionID, stun.BindingRequest)
	if err != nil {
		return "", 0, err
	}
	if err := req.NewTransactionID(); err != nil {
		return "", 0, err
	}

	start := time.Now()
	if _, err := conn.Write(req.Raw); err != nil {
		return "", 0, err
	}

	header := make([]byte, 20)
	if _, err := io.ReadFull(conn, header); err != nil {
		return "", 0, err
	}

	msgLen := int(binary.BigEndian.Uint16(header[2:4]))
	if msgLen < 0 || msgLen > maxTCPMessageSize {
		return "", 0, fmt.Errorf("invalid stun tcp length: %d", msgLen)
	}

	body := make([]byte, msgLen)
	if _, err := io.ReadFull(conn, body); err != nil {
		return "", 0, err
	}
	rtt = time.Since(start)

	raw := append(header, body...)

	var resp stun.Message
	resp.Raw = raw
	if err := resp.Decode(); err != nil {
		return "", 0, err
	}
	if resp.Type.Method != stun.MethodBinding {
		return "", 0, errors.New("unexpected stun method")
	}
	if resp.Type.Class != stun.ClassSuccessResponse {
		return "", 0, errors.New("unexpected stun class")
	}

	xor := &stun.XORMappedAddress{}
	if err := xor.GetFrom(&resp); err == nil {
		return xor.String(), rtt, nil
	}

	mapped := &stun.MappedAddress{}
	if err := mapped.GetFrom(&resp); err == nil {
		return mapped.String(), rtt, nil
	}

	return "", rtt, errors.New("no mapped address found")
}
