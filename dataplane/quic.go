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
	"github.com/miopunch/miopunch/internal/tlsutil"
)

const dataALPN = "miopunch-data"

func dialQUIC(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, payload []byte, em *event.Emitter) error {
	if raddr == nil {
		return fmt.Errorf("quic requires remote addr")
	}
	defer listenConn.Close()

	tlsConfig, err := tlsutil.NewClientTLSConfig("", "", "", raddr.String())
	if err != nil {
		return err
	}
	tlsConfig.NextProtos = []string{dataALPN}

	c, err := quic.Dial(ctx, listenConn, raddr, tlsConfig, &quic.Config{
		HandshakeIdleTimeout: 20 * time.Second,
		MaxIdleTimeout:       30 * time.Second,
		KeepAlivePeriod:      10 * time.Second,
	})
	if err != nil {
		return err
	}
	defer c.CloseWithError(0, "")

	if err := applyQUICCC(cfg, c); err != nil {
		return err
	}

	stream, err := c.OpenStreamSync(ctx)
	if err != nil {
		return err
	}
	defer stream.Close()

	if err := stream.SetDeadline(time.Now().Add(15 * time.Second)); err != nil {
		return err
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
	defer listenConn.Close()

	tlsConfig, err := tlsutil.NewServerTLSConfig("", "", "")
	if err != nil {
		return err
	}
	tlsConfig.NextProtos = []string{dataALPN}

	quicListener, err := quic.Listen(listenConn, tlsConfig, &quic.Config{
		HandshakeIdleTimeout: 20 * time.Second,
		MaxIdleTimeout:       30 * time.Second,
		KeepAlivePeriod:      10 * time.Second,
	})
	if err != nil {
		return err
	}
	defer quicListener.Close()

	c, err := quicListener.Accept(ctx)
	if err != nil {
		return err
	}
	defer c.CloseWithError(0, "")

	if err := applyQUICCC(cfg, c); err != nil {
		return err
	}

	stream, err := c.AcceptStream(ctx)
	if err != nil {
		return err
	}
	defer stream.Close()

	if err := stream.SetDeadline(time.Now().Add(15 * time.Second)); err != nil {
		return err
	}
	req, err := readFrame(stream, 64*1024)
	if err != nil {
		return err
	}
	resp := append([]byte("ok:"), req...)
	if err := writeFrame(stream, resp); err != nil {
		return err
	}

	emitPayloadExchanged(em, cfg, len(req), "quic-go")

	// Wait for the dialer side to close the connection before closing the underlying UDP socket.
	// Closing too early may surface as "Application error 0x0 (remote)" on the visitor.
	select {
	case <-ctx.Done():
	case <-c.Context().Done():
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
