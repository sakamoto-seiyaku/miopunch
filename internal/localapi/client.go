package localapi

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"strings"
	"time"
)

const (
	rpcVersion = "2.0"

	channelRPC    = "rpc"
	channelEvents = "events"
	channelShell  = "shell"
)

type Client struct {
	addr Addr
}

func NewClient(addr Addr) (*Client, error) {
	if addr.Transport == "" {
		return nil, fmt.Errorf("localapi transport is required")
	}
	if strings.TrimSpace(addr.Path) == "" {
		return nil, fmt.Errorf("localapi path is required")
	}
	return &Client{addr: addr}, nil
}

func (c *Client) ProbeStatus(ctx context.Context) error {
	ctx, cancel := context.WithTimeout(ctx, 300*time.Millisecond)
	defer cancel()
	_, err := c.GetStatus(ctx)
	return err
}

func (c *Client) GetStatus(ctx context.Context) (StatusResponse, error) {
	var status StatusResponse
	if err := c.call(ctx, "status", nil, &status); err != nil {
		return StatusResponse{}, err
	}
	return status, nil
}

func (c *Client) GetSnapshot(ctx context.Context) (Snapshot, error) {
	var snapshot Snapshot
	if err := c.call(ctx, "snapshot", nil, &snapshot); err != nil {
		return Snapshot{}, err
	}
	return snapshot, nil
}

func (c *Client) Action(ctx context.Context, action string, args any) (ActionResult, error) {
	req := ActionRequest{Action: strings.TrimSpace(action)}
	if args != nil {
		data, err := json.Marshal(args)
		if err != nil {
			return ActionResult{}, fmt.Errorf("marshal action args: %w", err)
		}
		req.Args = data
	}
	var result ActionResult
	if err := c.call(ctx, "action", req, &result); err != nil {
		return ActionResult{}, err
	}
	return result, nil
}

func (c *Client) OpenEvents(ctx context.Context) (io.ReadCloser, error) {
	conn, reader, err := c.openChannel(ctx, channelPreface{
		Version: protocolVersion,
		Channel: channelEvents,
	})
	if err != nil {
		return nil, err
	}
	return &bufferedConn{Conn: conn, reader: reader}, nil
}

func (c *Client) DialShell(ctx context.Context, shellSessionID string) (io.ReadWriteCloser, error) {
	conn, reader, err := c.openChannel(ctx, channelPreface{
		Version:        protocolVersion,
		Channel:        channelShell,
		ShellSessionID: strings.TrimSpace(shellSessionID),
	})
	if err != nil {
		return nil, err
	}
	return &bufferedConn{Conn: conn, reader: reader}, nil
}

func (c *Client) call(ctx context.Context, method string, params any, out any) error {
	conn, reader, err := c.openChannel(ctx, channelPreface{
		Version: protocolVersion,
		Channel: channelRPC,
	})
	if err != nil {
		return err
	}
	defer func() { _ = conn.Close() }()

	request := rpcRequest{
		JSONRPC: rpcVersion,
		ID:      mustMarshalRaw("1"),
		Method:  method,
	}
	if params != nil {
		request.Params = mustMarshalRaw(params)
	}
	if err := setConnDeadline(conn, ctx); err != nil {
		return fmt.Errorf("set rpc deadline: %w", err)
	}
	defer clearConnDeadline(conn)
	if err := json.NewEncoder(conn).Encode(request); err != nil {
		return fmt.Errorf("write rpc request: %w", err)
	}

	var response rpcResponse
	if err := json.NewDecoder(reader).Decode(&response); err != nil {
		if ctxErr := ctx.Err(); ctxErr != nil && isConnDeadlineError(err) {
			return ctxErr
		}
		return protocolError(err, "decode rpc response")
	}
	if response.JSONRPC != rpcVersion {
		return &UnexpectedStatusError{Problem: "unsupported rpc version"}
	}
	if response.Error != nil {
		return decodeRPCError(response.Error)
	}
	if out == nil {
		return nil
	}
	if len(response.Result) == 0 {
		return &UnexpectedStatusError{Problem: "missing rpc result"}
	}
	if err := json.Unmarshal(response.Result, out); err != nil {
		return protocolError(err, "decode rpc result")
	}
	return nil
}

func (c *Client) openChannel(ctx context.Context, preface channelPreface) (net.Conn, *bufio.Reader, error) {
	conn, err := c.dial(ctx)
	if err != nil {
		return nil, nil, err
	}
	reader := bufio.NewReader(conn)
	if err := json.NewEncoder(conn).Encode(preface); err != nil {
		_ = conn.Close()
		return nil, nil, fmt.Errorf("write channel preface: %w", err)
	}
	return conn, reader, nil
}

func (c *Client) dial(ctx context.Context) (net.Conn, error) {
	dial, err := dialContextForAddr(c.addr)
	if err != nil {
		return nil, err
	}
	return dial(ctx, "", "")
}

func decodeRPCError(in *rpcError) error {
	if in == nil {
		return &UnexpectedStatusError{Problem: "missing rpc error"}
	}
	if len(in.Data) == 0 {
		return &UnexpectedStatusError{Problem: in.Message}
	}
	var resp ErrorResponse
	if err := json.Unmarshal(in.Data, &resp); err != nil {
		return protocolError(err, "decode rpc error data")
	}
	return &APIError{Response: resp}
}

func protocolError(err error, problem string) error {
	if err == nil {
		return &UnexpectedStatusError{Problem: problem}
	}
	if strings.TrimSpace(problem) == "" {
		return &UnexpectedStatusError{Problem: err.Error()}
	}
	return &UnexpectedStatusError{Problem: problem + ": " + err.Error()}
}

func setConnDeadline(conn net.Conn, ctx context.Context) error {
	if conn == nil || ctx == nil {
		return nil
	}
	deadline, ok := ctx.Deadline()
	if !ok {
		return nil
	}
	return conn.SetDeadline(deadline)
}

func clearConnDeadline(conn net.Conn) {
	if conn == nil {
		return
	}
	_ = conn.SetDeadline(time.Time{})
}

func isConnDeadlineError(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return true
	}
	var netErr net.Error
	return errors.As(err, &netErr) && netErr.Timeout()
}

func trimJSONLine(line []byte) []byte {
	return []byte(strings.TrimSpace(string(line)))
}

type bufferedConn struct {
	net.Conn
	reader *bufio.Reader
}

func (c *bufferedConn) Read(p []byte) (int, error) {
	if c == nil || c.reader == nil {
		return 0, io.EOF
	}
	return c.reader.Read(p)
}
