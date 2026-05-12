//go:build windows

package shelltarget

import (
	"bytes"
	"context"
	"strings"
	"time"
)

type conPTYSmokeReadResult struct {
	payload []byte
	n       int
	err     error
	at      time.Time
}

type conPTYSmokeWriteResult struct {
	n   int
	err error
	at  time.Time
}

type conPTYSmokeWaitResult struct {
	err error
	at  time.Time
}

// RunConPTYSmoke runs a bounded Windows ConPTY diagnostic using the production backend.
func RunConPTYSmoke(ctx context.Context, req ConPTYSmokeRequest) ConPTYSmokeResult {
	startedAt := time.Now()
	timeout := conPTYSmokeTimeout(req.Timeout)
	res := ConPTYSmokeResult{
		Application: strings.TrimSpace(req.Application),
		Args:        append([]string(nil), req.Args...),
		TimeoutMS:   timeout.Milliseconds(),
	}
	if res.Application == "" {
		res.StartErr = "empty application"
		res.DurationMS = time.Since(startedAt).Milliseconds()
		return res
	}

	ptySess, err := startConPTY(res.Application, req.Args, req.Cols, req.Rows)
	if err != nil {
		res.StartErr = err.Error()
		res.DurationMS = time.Since(startedAt).Milliseconds()
		return res
	}
	res.Started = true
	res.PID = ptySess.procID
	res.CommandLine = ptySess.cmdline

	readCh := make(chan conPTYSmokeReadResult, 16)
	go func() {
		for {
			buf := make([]byte, 32*1024)
			n, err := ptySess.Read(buf)
			payload := append([]byte(nil), buf[:n]...)
			readCh <- conPTYSmokeReadResult{payload: payload, n: n, err: err, at: time.Now()}
			if err != nil {
				return
			}
		}
	}()

	waitCh := make(chan conPTYSmokeWaitResult, 1)
	go func() {
		waitCh <- conPTYSmokeWaitResult{err: ptySess.Wait(), at: time.Now()}
	}()

	var (
		timeoutTimer = time.NewTimer(timeout)
		writeTimer   *time.Timer
		writeC       <-chan time.Time
		writeCh      chan conPTYSmokeWriteResult
		readPayload  bytes.Buffer
		idleTimer    *time.Timer
		idleC        <-chan time.Time
		closed       bool
	)
	defer timeoutTimer.Stop()
	if len(req.Input) > 0 {
		writeDelay := req.WriteDelay
		if writeDelay < 0 {
			writeDelay = 0
		}
		writeTimer = time.NewTimer(writeDelay)
		writeC = writeTimer.C
		defer writeTimer.Stop()
	}

	closePTY := func() {
		if closed {
			return
		}
		closed = true
		_ = ptySess.Close()
	}
	recordRead := func(read conPTYSmokeReadResult, afterClose bool) {
		if read.n > 0 {
			_, _ = readPayload.Write(read.payload)
		}
		res.ReadReturned = true
		res.ReadAfterClose = afterClose
		res.ReadChunks++
		res.ReadN += read.n
		res.ReadErr = conPTYSmokeErrString(read.err)
		if res.ReadAfterMS == 0 {
			res.ReadAfterMS = read.at.Sub(startedAt).Milliseconds()
		}
		res.ReadLastAfterMS = read.at.Sub(startedAt).Milliseconds()
		res.PreviewText, res.PreviewHex = conPTYSmokePreview(readPayload.Bytes())
	}
	recordWait := func(wait conPTYSmokeWaitResult) {
		res.WaitReturned = true
		res.WaitErr = conPTYSmokeErrString(wait.err)
		res.WaitAfterMS = wait.at.Sub(startedAt).Milliseconds()
	}
	drainAfterClose := func() {
		closePTY()
		drainTimer := time.NewTimer(2 * time.Second)
		defer drainTimer.Stop()
		for !(res.ReadReturned && res.WaitReturned) {
			select {
			case read := <-readCh:
				if !res.ReadReturned {
					recordRead(read, true)
				}
			case wait := <-waitCh:
				if !res.WaitReturned {
					recordWait(wait)
				}
			case write := <-writeCh:
				if writeCh != nil && !res.WriteReturned {
					res.WriteReturned = true
					res.WriteN = write.n
					res.WriteErr = conPTYSmokeErrString(write.err)
					res.WriteAfterMS = write.at.Sub(startedAt).Milliseconds()
				}
			case <-drainTimer.C:
				return
			}
		}
	}

	stopIdleTimer := func() {
		if idleTimer == nil {
			return
		}
		if !idleTimer.Stop() {
			select {
			case <-idleTimer.C:
			default:
			}
		}
		idleTimer = nil
		idleC = nil
	}
	startIdleTimer := func(delay time.Duration) {
		stopIdleTimer()
		idleTimer = time.NewTimer(delay)
		idleC = idleTimer.C
	}
	defer stopIdleTimer()

	for {
		select {
		case <-ctx.Done():
			res.ReadTimedOut = true
			res.ReadErr = ctx.Err().Error()
			drainAfterClose()
			res.DurationMS = time.Since(startedAt).Milliseconds()
			return res
		case <-timeoutTimer.C:
			res.ReadTimedOut = true
			drainAfterClose()
			res.DurationMS = time.Since(startedAt).Milliseconds()
			return res
		case <-writeC:
			res.WriteAttempted = true
			res.WriteRequestedBytes = len(req.Input)
			writeCh = make(chan conPTYSmokeWriteResult, 1)
			go func() {
				n, err := ptySess.Write(req.Input)
				writeCh <- conPTYSmokeWriteResult{n: n, err: err, at: time.Now()}
			}()
			writeC = nil
		case write := <-writeCh:
			if writeCh != nil && !res.WriteReturned {
				res.WriteReturned = true
				res.WriteN = write.n
				res.WriteErr = conPTYSmokeErrString(write.err)
				res.WriteAfterMS = write.at.Sub(startedAt).Milliseconds()
			}
		case read := <-readCh:
			recordRead(read, false)
			if read.err != nil || res.WaitReturned {
				startIdleTimer(250 * time.Millisecond)
			}
		case wait := <-waitCh:
			recordWait(wait)
			waitCh = nil
			// Short-lived probes can exit before all buffered pipe output is read.
			// Wait briefly for the read loop to drain before closing the ConPTY.
			startIdleTimer(250 * time.Millisecond)
		case <-idleC:
			closePTY()
			drainAfterClose()
			res.DurationMS = time.Since(startedAt).Milliseconds()
			return res
		}
	}
}
