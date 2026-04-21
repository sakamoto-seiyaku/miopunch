package localapi

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/gorilla/websocket"

	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/task"
)

type Client struct {
	addr       Addr
	httpClient *http.Client
}

type APIError struct {
	StatusCode int
	Response   ErrorResponse
}

func (e *APIError) Error() string {
	msg := strings.TrimSpace(e.Response.Message)
	if msg == "" {
		msg = "localapi error"
	}
	return fmt.Sprintf("%s (http=%d exit_code=%d reason_code=%s)", msg, e.StatusCode, e.Response.ExitCode, e.Response.ReasonCode)
}

func NewClient(addr Addr) (*Client, error) {
	dial, err := dialContextForAddr(addr)
	if err != nil {
		return nil, err
	}

	tr := &http.Transport{
		DialContext: dial,
	}

	return &Client{
		addr:       addr,
		httpClient: &http.Client{Transport: tr},
	}, nil
}

func (c *Client) ProbeStatus(ctx context.Context) error {
	ctx, cancel := context.WithTimeout(ctx, 300*time.Millisecond)
	defer cancel()

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, "http://"+poc.LocalAPIHost+"/api/v0/status", nil)
	if err != nil {
		return err
	}
	req.Host = poc.LocalAPIHost

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return err
	}
	_, _ = io.Copy(io.Discard, resp.Body)
	_ = resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("unexpected status: %d", resp.StatusCode)
	}
	return nil
}

type PeersResponse struct {
	Peers []Peer `json:"peers"`
}

type Peer struct {
	PeerID string `json:"peer_id"`
}

func (c *Client) GetPeers(ctx context.Context) (PeersResponse, error) {
	var resp PeersResponse
	if err := c.doJSON(ctx, http.MethodGet, "/api/v0/peers", nil, &resp); err != nil {
		return PeersResponse{}, err
	}
	return resp, nil
}

func (c *Client) CreateTask(ctx context.Context, kind string, args any) (task.Task, error) {
	reqBody := map[string]any{
		"kind": kind,
	}
	if args != nil {
		reqBody["args"] = args
	}

	var created task.Task
	if err := c.doJSON(ctx, http.MethodPost, "/api/v0/tasks", reqBody, &created); err != nil {
		return task.Task{}, err
	}
	return created, nil
}

func (c *Client) GetTask(ctx context.Context, taskID string) (task.Task, error) {
	var t task.Task
	if err := c.doJSON(ctx, http.MethodGet, "/api/v0/tasks/"+url.PathEscape(taskID), nil, &t); err != nil {
		return task.Task{}, err
	}
	return t, nil
}

func (c *Client) OpenTaskEvents(ctx context.Context, taskID string) (io.ReadCloser, error) {
	resp, err := c.doRaw(ctx, http.MethodGet, "/api/v0/tasks/"+url.PathEscape(taskID)+"/events", nil)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode/100 == 2 {
		return resp.Body, nil
	}
	defer func() { _ = resp.Body.Close() }()

	apiErr, err := decodeAPIError(resp)
	if err != nil {
		return nil, err
	}
	return nil, apiErr
}

func (c *Client) OpenEvents(ctx context.Context) (io.ReadCloser, error) {
	resp, err := c.doRaw(ctx, http.MethodGet, "/api/v0/events", nil)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode/100 == 2 {
		return resp.Body, nil
	}
	defer func() { _ = resp.Body.Close() }()

	apiErr, err := decodeAPIError(resp)
	if err != nil {
		return nil, err
	}
	return nil, apiErr
}

func (c *Client) DialTaskWS(ctx context.Context, taskID string) (*websocket.Conn, *http.Response, error) {
	dial, err := dialContextForAddr(c.addr)
	if err != nil {
		return nil, nil, err
	}

	d := websocket.Dialer{
		NetDialContext: dial,
		Subprotocols:   []string{shSubprotocolV0},
	}
	u := url.URL{
		Scheme: "ws",
		Host:   poc.LocalAPIHost,
		Path:   "/api/v0/tasks/" + taskID + "/ws",
	}
	return d.DialContext(ctx, u.String(), nil)
}

func (c *Client) doJSON(ctx context.Context, method string, path string, reqBody any, respBody any) error {
	var body io.Reader
	if reqBody != nil {
		b, err := json.Marshal(reqBody)
		if err != nil {
			return err
		}
		body = bytes.NewReader(b)
	}

	resp, err := c.doRaw(ctx, method, path, body)
	if err != nil {
		return err
	}
	defer func() { _ = resp.Body.Close() }()

	if resp.StatusCode/100 == 2 {
		return json.NewDecoder(resp.Body).Decode(respBody)
	}

	apiErr, err := decodeAPIError(resp)
	if err != nil {
		return err
	}
	return apiErr
}

func (c *Client) doRaw(ctx context.Context, method string, path string, body io.Reader) (*http.Response, error) {
	u := "http://" + poc.LocalAPIHost + path
	req, err := http.NewRequestWithContext(ctx, method, u, body)
	if err != nil {
		return nil, err
	}
	req.Host = poc.LocalAPIHost
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	return c.httpClient.Do(req)
}

func decodeAPIError(resp *http.Response) (*APIError, error) {
	var er ErrorResponse
	if err := json.NewDecoder(resp.Body).Decode(&er); err != nil {
		return nil, err
	}
	return &APIError{
		StatusCode: resp.StatusCode,
		Response:   er,
	}, nil
}
