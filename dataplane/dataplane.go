package dataplane

import (
	"context"
	"fmt"
	"net"
	"time"

	"github.com/miopunch/miopunch/event"
)

// DialAndExchange establishes the selected data plane over the already-working UDP path,
// sends a payload, and expects an "ok:<payload>" response.
//
// This is intended for the "visitor"/dialer side in the lab.
func DialAndExchange(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, payload []byte, em *event.Emitter) error {
	if err := cfg.Validate(); err != nil {
		return err
	}
	switch cfg.Proto {
	case ProtocolKCP:
		return dialKCP(ctx, cfg, listenConn, raddr, payload, em)
	case ProtocolQUIC:
		return dialQUIC(ctx, cfg, listenConn, raddr, payload, em)
	default:
		return fmt.Errorf("unknown data proto: %q", cfg.Proto)
	}
}

// ServeAndExchange accepts / serves the selected data plane over the already-working UDP path,
// reads a payload, and responds with "ok:<payload>".
//
// This is intended for the "client"/acceptor side in the lab.
func ServeAndExchange(ctx context.Context, cfg Config, listenConn *net.UDPConn, raddr *net.UDPAddr, em *event.Emitter) error {
	if err := cfg.Validate(); err != nil {
		return err
	}
	switch cfg.Proto {
	case ProtocolKCP:
		return serveKCP(ctx, cfg, listenConn, raddr, em)
	case ProtocolQUIC:
		return serveQUIC(ctx, cfg, listenConn, em)
	default:
		return fmt.Errorf("unknown data proto: %q", cfg.Proto)
	}
}

func emitPayloadExchanged(em *event.Emitter, cfg Config, bytes int, impl string) {
	if em == nil {
		return
	}
	cfg.Normalize()

	kvs := map[string]any{
		"bytes":      bytes,
		"impl":       impl,
		"data_proto": string(cfg.Proto),
	}
	if cfg.Proto == ProtocolQUIC {
		kvs["quic_cc"] = string(cfg.QuicCC)
		if cfg.QuicCC == QUICCCBrutal {
			kvs["brutal_up_bps"] = cfg.Brutal.UpBps
			kvs["brutal_down_bps"] = cfg.Brutal.DownBps
		}
	}

	em.Emit(event.Event{
		Stage: event.StageTransport,
		Kind:  event.KindOK,
		Name:  "transport.payload_exchanged",
		Msg:   string(cfg.Proto) + " payload exchanged",
		KVs:   kvs,
	})
}

func withDeadline(t time.Time, fn func() error) error {
	return fn()
}
