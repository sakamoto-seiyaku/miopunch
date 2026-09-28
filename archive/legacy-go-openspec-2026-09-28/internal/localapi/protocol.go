package localapi

import (
	"encoding/json"
	"fmt"
)

const protocolVersion = 1

type channelPreface struct {
	Version        int    `json:"version"`
	Channel        string `json:"channel"`
	ShellSessionID string `json:"shell_session_id,omitempty"`
}

type rpcRequest struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id"`
	Method  string          `json:"method"`
	Params  json.RawMessage `json:"params,omitempty"`
}

type rpcResponse struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id,omitempty"`
	Result  json.RawMessage `json:"result,omitempty"`
	Error   *rpcError       `json:"error,omitempty"`
}

type rpcError struct {
	Code    int             `json:"code"`
	Message string          `json:"message"`
	Data    json.RawMessage `json:"data,omitempty"`
}

type shellAck struct {
	OK    bool           `json:"ok"`
	Error *ErrorResponse `json:"error,omitempty"`
}

func mustMarshalRaw(value any) json.RawMessage {
	data, err := json.Marshal(value)
	if err != nil {
		panic(fmt.Sprintf("marshal localapi value: %v", err))
	}
	return data
}
