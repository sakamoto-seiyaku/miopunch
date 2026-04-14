package dataplane

import (
	"context"
	"errors"
	"fmt"
	"net"
	"time"

	"github.com/miopunch/miopunch/event"
	"github.com/miopunch/miopunch/internal/netutil"
)

func dialKCP(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, payload []byte, em *event.Emitter) error {
	if raddr == nil {
		return fmt.Errorf("kcp requires remote addr")
	}
	if listenConn == nil {
		return errors.New("kcp requires listen conn")
	}

	laddr, err := net.ResolveUDPAddr("udp", listenConn.LocalAddr().String())
	if err != nil {
		return err
	}
	_ = listenConn.Close()

	lConn, err := net.DialUDP("udp", laddr, raddr)
	if err != nil {
		return err
	}
	defer lConn.Close()

	c, err := netutil.NewKCPConnFromUDP(lConn, true, raddr.String())
	if err != nil {
		return err
	}
	defer c.Close()

	if err := c.SetDeadline(time.Now().Add(15 * time.Second)); err != nil {
		return err
	}
	if err := writeFrame(c, payload); err != nil {
		return err
	}
	resp, err := readFrame(c, 64*1024)
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

	laddr, err := net.ResolveUDPAddr("udp", listenConn.LocalAddr().String())
	if err != nil {
		return err
	}
	_ = listenConn.Close()

	lConn, err := net.DialUDP("udp", laddr, raddr)
	if err != nil {
		return err
	}
	defer lConn.Close()

	c, err := netutil.NewKCPConnFromUDP(lConn, true, raddr.String())
	if err != nil {
		return err
	}
	defer c.Close()

	if err := c.SetDeadline(time.Now().Add(15 * time.Second)); err != nil {
		return err
	}
	req, err := readFrame(c, 64*1024)
	if err != nil {
		return err
	}
	resp := append([]byte("ok:"), req...)
	if err := writeFrame(c, resp); err != nil {
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
