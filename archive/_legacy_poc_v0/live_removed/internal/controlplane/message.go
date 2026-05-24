package controlplane

import (
	"encoding/json"
	"fmt"
)

const (
	ProtoVersionV0 = 0

	// POC v0: bounded flooding hop limit.
	HopLimitMax = 3
)

type Message struct {
	ProtoVersion int    `json:"proto_version"`
	Route        Route  `json:"route"`
	Signed       Signed `json:"signed"`
}

type Route struct {
	DstPeerID       string `json:"dst_peer_id"`
	MsgID           string `json:"msg_id"`
	HopLimit        int    `json:"hop_limit"`
	CreatedAtUnixMs int64  `json:"created_at_unix_ms"`
	ExpiresAtUnixMs int64  `json:"expires_at_unix_ms,omitempty"`
}

type Signed struct {
	SenderPeerID string          `json:"sender_peer_id"`
	Kind         string          `json:"kind"`
	InReplyTo    string          `json:"in_reply_to,omitempty"`
	Body         json.RawMessage `json:"body"`
	SigB64       string          `json:"sig_b64"`
}

func MarshalMessage(m Message) ([]byte, error) {
	data, err := json.Marshal(m)
	if err != nil {
		return nil, fmt.Errorf("marshal controlplane message: %w", err)
	}
	return data, nil
}

func UnmarshalMessage(data []byte) (Message, error) {
	var m Message
	if err := json.Unmarshal(data, &m); err != nil {
		return Message{}, fmt.Errorf("unmarshal controlplane message: %w", err)
	}
	return m, nil
}
