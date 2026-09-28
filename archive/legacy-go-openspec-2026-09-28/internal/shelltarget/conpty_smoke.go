package shelltarget

import (
	"encoding/hex"
	"strconv"
	"time"
	"unicode/utf8"
)

// ConPTYSmokeRequest describes a single Windows ConPTY diagnostic run.
type ConPTYSmokeRequest struct {
	Application string
	Args        []string
	Input       []byte
	WriteDelay  time.Duration
	Timeout     time.Duration
	Cols        int
	Rows        int
}

// ConPTYSmokeResult contains bounded evidence from one ConPTY diagnostic run.
type ConPTYSmokeResult struct {
	Application         string   `json:"application"`
	Args                []string `json:"args,omitempty"`
	CommandLine         string   `json:"command_line,omitempty"`
	PID                 uint32   `json:"pid,omitempty"`
	Started             bool     `json:"started"`
	StartErr            string   `json:"start_err,omitempty"`
	TimeoutMS           int64    `json:"timeout_ms"`
	DurationMS          int64    `json:"duration_ms"`
	ReadReturned        bool     `json:"read_returned"`
	ReadAfterClose      bool     `json:"read_after_close,omitempty"`
	ReadTimedOut        bool     `json:"read_timed_out,omitempty"`
	ReadChunks          int      `json:"read_chunks,omitempty"`
	ReadN               int      `json:"read_n"`
	ReadErr             string   `json:"read_err,omitempty"`
	ReadAfterMS         int64    `json:"read_after_ms,omitempty"`
	ReadLastAfterMS     int64    `json:"read_last_after_ms,omitempty"`
	PreviewText         string   `json:"preview_text,omitempty"`
	PreviewHex          string   `json:"preview_hex,omitempty"`
	WriteAttempted      bool     `json:"write_attempted,omitempty"`
	WriteRequestedBytes int      `json:"write_requested_bytes,omitempty"`
	WriteReturned       bool     `json:"write_returned,omitempty"`
	WriteN              int      `json:"write_n,omitempty"`
	WriteErr            string   `json:"write_err,omitempty"`
	WriteAfterMS        int64    `json:"write_after_ms,omitempty"`
	WaitReturned        bool     `json:"wait_returned,omitempty"`
	WaitErr             string   `json:"wait_err,omitempty"`
	WaitAfterMS         int64    `json:"wait_after_ms,omitempty"`
}

func conPTYSmokeTimeout(timeout time.Duration) time.Duration {
	if timeout > 0 {
		return timeout
	}
	return 6 * time.Second
}

func conPTYSmokePreview(payload []byte) (text string, hexText string) {
	if len(payload) > 512 {
		payload = payload[:512]
	}
	hexText = hex.EncodeToString(payload)
	if utf8.Valid(payload) {
		return strconv.QuoteToASCII(string(payload)), hexText
	}
	return "", hexText
}

func conPTYSmokeErrString(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}
