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
	"sync"
	"time"

	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/poc"
	pocruntime "github.com/miopunch/miopunch/internal/pocv1/runtime"
)

type Server struct {
	mode      ListenMode
	runtime   *pocruntime.Runtime
	startedAt time.Time

	ctx    context.Context
	cancel context.CancelFunc
	wg     sync.WaitGroup
}

func NewServer(mode ListenMode, runtime *pocruntime.Runtime) *Server {
	if runtime == nil {
		panic("localapi runtime is required")
	}
	ctx, cancel := context.WithCancel(context.Background())
	return &Server{
		mode:      mode,
		runtime:   runtime,
		startedAt: time.Now().UTC(),
		ctx:       ctx,
		cancel:    cancel,
	}
}

func (s *Server) Close() error {
	if s == nil {
		return nil
	}
	s.cancel()
	s.wg.Wait()
	return nil
}

func (s *Server) Serve(listener net.Listener) error {
	if listener == nil {
		return errors.New("nil listener")
	}
	for {
		conn, err := listener.Accept()
		if err != nil {
			if s.ctx.Err() != nil {
				return nil
			}
			if errors.Is(err, net.ErrClosed) {
				return nil
			}
			return err
		}

		s.wg.Add(1)
		go func() {
			defer s.wg.Done()
			s.handleConn(conn)
		}()
	}
}

func (s *Server) handleConn(conn net.Conn) {
	defer func() { _ = conn.Close() }()

	reader := bufio.NewReader(conn)
	line, err := reader.ReadBytes('\n')
	if err != nil {
		return
	}

	var preface channelPreface
	if err := json.Unmarshal(trimJSONLine(line), &preface); err != nil {
		s.writeRPCProtocolError(conn, nil, newProtocolProblem("invalid channel preface", err))
		return
	}
	if preface.Version != protocolVersion {
		s.writeRPCProtocolError(conn, nil, newProblemResponse(
			"localapi",
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"unsupported localapi protocol version",
			[]poc.Fact{{Message: fmt.Sprintf("version=%d", preface.Version)}},
			[]poc.Suggestion{{Message: "retry with matching miopunch binaries"}},
		))
		return
	}

	switch preface.Channel {
	case channelRPC:
		s.serveRPC(conn, reader)
	case channelEvents:
		s.serveEvents(conn)
	case channelShell:
		s.serveShell(&bufferedConn{Conn: conn, reader: reader}, preface)
	default:
		s.writeRPCProtocolError(conn, nil, newProblemResponse(
			"localapi",
			poc.ReasonCodeBadRequest,
			poc.ExitCodeBadRequest,
			"unsupported localapi channel",
			[]poc.Fact{{Message: "channel=" + strings.TrimSpace(preface.Channel)}},
			[]poc.Suggestion{{Message: "retry with a supported localapi client"}},
		))
	}
}

func (s *Server) serveRPC(conn net.Conn, reader *bufio.Reader) {
	decoder := json.NewDecoder(reader)
	encoder := json.NewEncoder(conn)
	for {
		var request rpcRequest
		if err := decoder.Decode(&request); err != nil {
			if errors.Is(err, io.EOF) {
				return
			}
			_ = encoder.Encode(rpcResponse{
				JSONRPC: rpcVersion,
				Error: &rpcError{
					Code:    -32700,
					Message: "parse error",
					Data:    mustMarshalRaw(newProtocolProblem("decode rpc request", err)),
				},
			})
			return
		}

		response := s.handleRPCRequest(request)
		if err := encoder.Encode(response); err != nil {
			return
		}
	}
}

func (s *Server) handleRPCRequest(request rpcRequest) (response rpcResponse) {
	method := strings.TrimSpace(request.Method)
	startedAt := time.Now()
	logutil.Debugf("localapi rpc start: method=%s", method)
	defer func() {
		elapsedMs := time.Since(startedAt).Milliseconds()
		if response.Error != nil {
			logutil.Debugf(
				"localapi rpc done: method=%s elapsed_ms=%d error=%s",
				method,
				elapsedMs,
				response.Error.Message,
			)
			return
		}
		logutil.Debugf("localapi rpc done: method=%s elapsed_ms=%d", method, elapsedMs)
	}()

	response = rpcResponse{
		JSONRPC: rpcVersion,
		ID:      request.ID,
	}
	if request.JSONRPC != rpcVersion {
		response.Error = &rpcError{
			Code:    -32600,
			Message: "invalid request",
			Data: mustMarshalRaw(newProblemResponse(
				"localapi",
				poc.ReasonCodeBadRequest,
				poc.ExitCodeBadRequest,
				"unsupported rpc version",
				nil,
				[]poc.Suggestion{{Message: "retry with matching miopunch binaries"}},
			)),
		}
		return response
	}

	switch request.Method {
	case "status":
		status := s.runtime.Status()
		response.Result = mustMarshalRaw(StatusResponse{
			Version:   status.Version,
			StartedAt: status.StartedAt,
			UptimeMs:  status.UptimeMs,
			Mode:      s.mode,
		})
	case "snapshot":
		response.Result = mustMarshalRaw(s.runtime.Snapshot())
	case "set_log_level":
		var params LogLevelRequest
		if err := json.Unmarshal(request.Params, &params); err != nil {
			response.Error = &rpcError{
				Code:    -32602,
				Message: "invalid params",
				Data:    mustMarshalRaw(newProtocolProblem("decode log level params", err)),
			}
			return response
		}
		snapshot, problem := s.runtime.SetLogLevel(params.LogLevel)
		if problem != nil {
			resp := problem.ErrorResponse()
			response.Error = &rpcError{
				Code:    int(resp.ExitCode) * -1,
				Message: problem.Error(),
				Data:    mustMarshalRaw(resp),
			}
			return response
		}
		response.Result = mustMarshalRaw(snapshot)
	case "action":
		var params ActionRequest
		if err := json.Unmarshal(request.Params, &params); err != nil {
			response.Error = &rpcError{
				Code:    -32602,
				Message: "invalid params",
				Data:    mustMarshalRaw(newProtocolProblem("decode action params", err)),
			}
			return response
		}
		actionStartedAt := time.Now()
		logutil.Debugf("localapi action start: action=%s", strings.TrimSpace(params.Action))
		result, problem := s.runtime.Action(s.ctx, params.Action, params.Args)
		if problem != nil {
			resp := problem.ErrorResponse()
			logutil.Debugf(
				"localapi action done: action=%s elapsed_ms=%d stage=%s reason_code=%s exit_code=%d",
				strings.TrimSpace(params.Action),
				time.Since(actionStartedAt).Milliseconds(),
				resp.Stage,
				resp.ReasonCode,
				resp.ExitCode,
			)
			response.Error = &rpcError{
				Code:    int(resp.ExitCode) * -1,
				Message: problem.Error(),
				Data:    mustMarshalRaw(resp),
			}
			return response
		}
		logutil.Debugf(
			"localapi action done: action=%s elapsed_ms=%d stage=%s reason_code=%s exit_code=%d",
			strings.TrimSpace(params.Action),
			time.Since(actionStartedAt).Milliseconds(),
			result.Stage,
			result.ReasonCode,
			result.ExitCode,
		)
		response.Result = mustMarshalRaw(result)
	default:
		response.Error = &rpcError{
			Code:    -32601,
			Message: "method not found",
			Data: mustMarshalRaw(newProblemResponse(
				"localapi",
				poc.ReasonCodeBadRequest,
				poc.ExitCodeBadRequest,
				"unsupported rpc method",
				[]poc.Fact{{Message: "method=" + strings.TrimSpace(request.Method)}},
				[]poc.Suggestion{{Message: "retry with a supported localapi client"}},
			)),
		}
	}
	return response
}

func (s *Server) serveEvents(conn net.Conn) {
	subID, ch := s.runtime.Subscribe()
	defer s.runtime.Unsubscribe(subID)

	encoder := json.NewEncoder(conn)
	for {
		select {
		case <-s.ctx.Done():
			return
		case event, ok := <-ch:
			if !ok {
				return
			}
			if err := encoder.Encode(event); err != nil {
				return
			}
		}
	}
}

func (s *Server) serveShell(conn io.ReadWriteCloser, preface channelPreface) {
	if strings.TrimSpace(preface.ShellSessionID) == "" {
		return
	}
	_ = s.runtime.AttachShell(s.ctx, preface.ShellSessionID, conn)
}

func (s *Server) writeRPCProtocolError(conn net.Conn, id json.RawMessage, resp ErrorResponse) {
	_ = json.NewEncoder(conn).Encode(rpcResponse{
		JSONRPC: rpcVersion,
		ID:      id,
		Error: &rpcError{
			Code:    -32600,
			Message: resp.Message,
			Data:    mustMarshalRaw(resp),
		},
	})
}

func newProtocolProblem(message string, err error) ErrorResponse {
	facts := []poc.Fact{}
	if err != nil {
		facts = append(facts, poc.Fact{Message: "error=" + err.Error()})
	}
	return newProblemResponse(
		"localapi",
		poc.ReasonCodeBadRequest,
		poc.ExitCodeBadRequest,
		message,
		facts,
		[]poc.Suggestion{{Message: "retry with a supported localapi client"}},
	)
}

func newProblemResponse(
	stage string,
	reasonCode poc.ReasonCode,
	exitCode poc.ExitCode,
	message string,
	facts []poc.Fact,
	suggestions []poc.Suggestion,
) ErrorResponse {
	return ErrorResponse{
		Stage:       stage,
		ReasonCode:  reasonCode,
		ExitCode:    exitCode,
		Message:     strings.TrimSpace(message),
		Facts:       append([]poc.Fact(nil), facts...),
		Suggestions: append([]poc.Suggestion(nil), suggestions...),
	}
}
