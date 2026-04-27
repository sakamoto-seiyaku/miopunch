package dataplane

import (
	"context"
	"errors"
	"fmt"
	"net"
	"time"

	"github.com/miopunch/miopunch/event"
)

func dialKCP(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, payload []byte, em *event.Emitter) error {
	if raddr == nil {
		return fmt.Errorf("kcp requires remote addr")
	}
	if listenConn == nil {
		return errors.New("kcp requires listen conn")
	}

	sess, err := DialSession(ctx, cfg, listenConn, raddr, em)
	if err != nil {
		return err
	}
	defer sess.Close(CloseReasonDaemonShutdown)

	stream, err := sess.OpenStream(ctx, StreamOpen{Kind: StreamKindPayloadV0})
	if err != nil {
		return err
	}
	defer stream.Close()

	if conn, ok := stream.(interface{ SetDeadline(time.Time) error }); ok {
		_ = conn.SetDeadline(time.Now().Add(15 * time.Second))
		defer conn.SetDeadline(time.Time{})
	}
	if err := writeFrame(stream, payload); err != nil {
		return err
	}
	resp, err := readFrame(stream, 64*1024)
	if err != nil {
		return err
	}
	if string(resp) != "ok:"+string(payload) {
		return fmt.Errorf("unexpected response: %q", string(resp))
	}

	emitPayloadExchanged(em, cfg, len(payload), "kcp-go")
	return nil
}

func serveKCP(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, em *event.Emitter) error {
	if raddr == nil {
		return fmt.Errorf("kcp requires remote addr")
	}
	if listenConn == nil {
		return errors.New("kcp requires listen conn")
	}

	sess, err := ServeSession(ctx, cfg, listenConn, raddr, em)
	if err != nil {
		return err
	}
	defer sess.Close(CloseReasonDaemonShutdown)

	accepted, err := sess.AcceptStream(ctx)
	if err != nil {
		return err
	}
	defer accepted.Stream.Close()
	if accepted.Open.Kind != StreamKindPayloadV0 {
		return fmt.Errorf("unexpected stream kind: %q", accepted.Open.Kind)
	}

	if conn, ok := accepted.Stream.(interface{ SetDeadline(time.Time) error }); ok {
		_ = conn.SetDeadline(time.Now().Add(15 * time.Second))
		defer conn.SetDeadline(time.Time{})
	}
	req, err := readFrame(accepted.Stream, 64*1024)
	if err != nil {
		return err
	}
	resp := append([]byte("ok:"), req...)
	if err := writeFrame(accepted.Stream, resp); err != nil {
		return err
	}

	emitPayloadExchanged(em, cfg, len(req), "kcp-go")

	// Keep the UDP socket alive briefly so the dialer side can finish its exchange
	// without receiving ICMP "port unreachable" due to early close.
	select {
	case <-ctx.Done():
	case <-time.After(750 * time.Millisecond):
	}
	return nil
}
