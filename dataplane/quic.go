package dataplane

import (
	"context"
	"fmt"
	"net"
	"time"

	"github.com/apernet/quic-go"
	"github.com/apernet/quic-go/congestion"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/dataplane/congestion/bbr"
	"github.com/miopunch/miopunch/internal/dataplane/congestion/brutal"
)

const dataALPN = "miopunch-data"

func dialQUIC(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, payload []byte, em *event.Emitter) error {
	if raddr == nil {
		return fmt.Errorf("quic requires remote addr")
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

	emitPayloadExchanged(em, cfg, len(payload), "quic-go")
	return nil
}

func serveQUIC(ctx context.Context, cfg Config, listenConn *net.UDPConn, em *event.Emitter) error {
	sess, err := ServeSession(ctx, cfg, listenConn, nil, em)
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

	emitPayloadExchanged(em, cfg, len(req), "quic-go")

	// Wait for the dialer side to close the connection before closing the underlying UDP socket.
	// Closing too early may surface as "Application error 0x0 (remote)" on the visitor.
	select {
	case <-ctx.Done():
	case <-time.After(2 * time.Second):
	}
	return nil
}

func applyQUICCC(cfg Config, conn *quic.Conn) error {
	cfg.Normalize()
	switch cfg.QuicCC {
	case QUICCCBBR:
		conn.SetCongestionControl(bbr.NewBbrSender(bbr.DefaultClock{}, congestion.InitialPacketSize))
		return nil
	case QUICCCBrutal:
		conn.SetCongestionControl(brutal.NewBrutalSender(cfg.Brutal.UpBps))
		return nil
	default:
		return fmt.Errorf("unsupported quic cc: %q", cfg.QuicCC)
	}
}
