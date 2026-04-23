//go:build desktop

package main

import (
	"context"
	"fmt"

	"github.com/gorilla/websocket"

	"github.com/miopunch/miopunch/internal/desktopbridge"
)

type TerminalBridgeInfoResult struct {
	OK    bool                       `json:"ok"`
	Error *desktopbridge.BridgeError `json:"error,omitempty"`

	BaseURL     string `json:"base_url,omitempty"`
	Token       string `json:"token,omitempty"`
	Subprotocol string `json:"subprotocol,omitempty"`
}

func (a *App) TerminalBridgeInfo() TerminalBridgeInfoResult {
	if err := a.ensureTerminalBridge(); err != nil {
		return TerminalBridgeInfoResult{OK: false, Error: err}
	}

	a.mu.Lock()
	b := a.termBridge
	a.mu.Unlock()

	if b == nil {
		return TerminalBridgeInfoResult{
			OK: false,
			Error: &desktopbridge.BridgeError{
				Stage:   "desktop",
				Message: "terminal bridge is not running",
			},
		}
	}

	return TerminalBridgeInfoResult{
		OK:          true,
		BaseURL:     b.BaseURL(),
		Token:       b.Token(),
		Subprotocol: desktopbridge.ShellSubprotocolV0,
	}
}

func (a *App) ensureTerminalBridge() *desktopbridge.BridgeError {
	a.mu.Lock()
	existing := a.termBridge
	a.mu.Unlock()
	if existing != nil {
		return nil
	}

	bridge, err := desktopbridge.NewTerminalWSBridge(func(ctx context.Context, taskID string) (*websocket.Conn, error) {
		c, berr := a.localAPIClient()
		if berr != nil {
			return nil, fmt.Errorf("localapi not connected: %s", berr.ReasonCode)
		}

		conn, resp, err := c.DialTaskWS(ctx, taskID)
		if resp != nil {
			_ = resp.Body.Close()
		}
		return conn, err
	})
	if err != nil {
		return bridgeErrorFromErr(err)
	}

	a.mu.Lock()
	if a.termBridge == nil {
		a.termBridge = bridge
		a.mu.Unlock()
		return nil
	}
	a.mu.Unlock()

	_ = bridge.Close()
	return nil
}

func (a *App) closeTerminalBridge() {
	a.mu.Lock()
	b := a.termBridge
	a.termBridge = nil
	a.mu.Unlock()

	if b != nil {
		_ = b.Close()
	}
}
